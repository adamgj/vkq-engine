//! `Quake/sv_send.c` -- the server network message writer (Rust migration
//! Phase 7 M6, T6.3). Ported behind `Quake/sv_send_glue.c` under
//! `-Duse_rust_host`; `Quake/sv_send.c` remains the reference oracle until
//! Phase 9, and `rust/quake-ctest/tests/sv_send_differential.rs` runs the two
//! against each other.
//!
//! ## ADR-009 audit
//!
//! Every callee of `sv_send.c` was checked for reachable `Host_Error` /
//! `Host_EndGame`:
//!
//! * Raise-capable, therefore reached only through a `Host_Guard` trampoline
//!   in `Quake/sv_send_glue.c`: all `MSG_Write*` and `SZ_Write` (they reach
//!   `SZ_GetSpace`, `net_msg.c:488`), `PR_GetString`, `NUM_FOR_EDICT` /
//!   `EDICT_NUM` (`World_Glue_*`, shared with M3/M4), `SV_DropClient`
//!   (transitively, `host.c:590`), `Cmd_ExecuteString`, and `SV_SetIdealPitch`
//!   (whose plain name is T6.4's own `Host_Reraise` wrapper).
//! * Terminating but not longjmping, therefore called directly: `Sys_Error`
//!   (`sv_send.c:1096`; precedent `world.rs:32`, `sv_phys.rs:33`).
//! * No raise path at all, therefore called directly: `Mem_Alloc`,
//!   `Mem_Realloc`, `Mem_Free`, `q_strdup`, `Mod_LeafPVS`,
//!   `ED_FindFieldOffset`, `GetEdictFieldValue`, `SZ_Clear`, `AngleVectors`,
//!   `SV_ModelIndex`, `Con_Printf` / `Con_DWarning`, and the four net funnels
//!   `NET_CanSendMessage`, `NET_SendMessage`, `NET_SendUnreliableMessage` and
//!   `NET_SendToAll` -- their loopback and datagram drivers bottom out in
//!   `Sys_Error` only (`net_loop.c:183`, `net_dgrm.c:344`, `net_dgrm.c:663`),
//!   with no `Host_Error` anywhere on the send path.
//!
//! `sv_send.c` calls no `SV_StartSound`, so that raise site does not arise
//! here. `Host_Reraise` is never called from Rust; only the glue calls it.
//!
//! ## ADR-010
//!
//! Every arithmetic expression keeps C's exact width and association. The
//! places where that matters are marked `// COMPAT: ADR-010` at the site.
//! The recurring traps are `qcvm->time`, which is a `double`, so
//! `qcvm->time - ent->lastthink` and `ent->v.nextthink - qcvm->time` evaluate
//! in `f64` before any truncation; `Q_rint`, which promotes to `double`;
//! `ENTALPHA_ENCODE`, whose `Q_rint` boundary lands exactly on 64.5 for the
//! `f32` immediately below 0.25 and would round the other way if the
//! multiply were done in `f64`; and `sqrt`, taken from the platform libm via
//! `quake_c_sys::libm` rather than `f64::sqrt`.
//!
//! ## ADR-005
//!
//! `sv_send.c` has no floating-point conversion specifier at all. Its three
//! `Con_*` messages and its one `Sys_Error` use `%i` and `%d` only, and all
//! four stay in `Quake/sv_send_glue.c`, so no Rust formatter is involved.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cvar_cmd as cc;
use quake_c_sys::sv_main as m;
use quake_c_sys::sv_phys as ph;
use quake_c_sys::sv_send as g;
use quake_c_sys::world as w;
use quake_types::bspfile::CONTENTS_SOLID;
use quake_types::host::{
    Client, ClientStatic, DeltaFrame, DeltaFrameEnt, EntityNumState, MAX_CL_STATS, NUM_PING_TIMES,
    PRESPAWN_AMBIENTS, PRESPAWN_BASELINES, PRESPAWN_DONE, PRESPAWN_FLUSH, PRESPAWN_MODELS,
    PRESPAWN_PARTICLES, PRESPAWN_SIGNONMSG, PRESPAWN_SOUNDS, PRESPAWN_STATICS,
};
use quake_types::model_mem::{MNode, QModel};
use quake_types::net::{SizeBuf, DATAGRAM_MTU, MAX_DATAGRAM};
use quake_types::progs::{etype, Edict, EntityState, GlobalVars, QcVm, MAX_EDICTS};

/// A `Host_Guard` status: `HOST_GUARD_OK` (0) or the code the guarded frame
/// caught. Non-zero must be returned to `Quake/sv_send_glue.c` untouched.
type Raise = c_int;

/// Propagate a non-zero `Host_Guard` status to the caller, abandoning the
/// rest of the body exactly where C's `longjmp` would have left it.
macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// ADR-011 mirror-typed engine externs.
//
// `quake-c-sys` has no `[dependencies]` section, so it cannot name the
// `quake-types` mirrors; those externs live here instead, exactly as T6.4's
// `sv_user.rs` and T6.5's `sv_main.rs` do. Everything untyped (cvars, the
// glue trampolines, the net funnels) is declared in
// `quake-c-sys/src/sv_send.rs` and reached through `g::`.

/// `glquake.h:600` `devstats_t`.
#[repr(C)]
struct DevStats {
    packetsize: c_int,
    edicts: c_int,
    visedicts: c_int,
    efrags: c_int,
    tempents: c_int,
    beams: c_int,
    dlights: c_int,
}

/// `glquake.h:612` `overflowtimes_t`.
#[repr(C)]
struct OverflowTimes {
    packetsize: f64,
    efrags: f64,
    beams: f64,
    varstring: f64,
}

// `sv`/`svs` became Rust-owned storage at T6.6 (ADR-007 row closed), so they
// are a sibling module's items now rather than C externs.
use crate::sv_main::{sv, svs};

extern "C" {
    /// `Quake/host.c` -- the client the writer is currently serving.
    /// `SV_SendClientDatagram` assigns it (`sv_send.c:1762`).
    static mut host_client: *mut Client;
    /// `Quake/cl_main.c`. Only `SV_UpdateToReliableMessages` reads it, and
    /// only through the `LERP_BANDAID` strip test, which stays in the glue.
    #[allow(dead_code)]
    static mut cls: ClientStatic;
    /// `Quake/protocol.c` -- `entity_state_t nullentitystate;` (note: not
    /// all-zero). Compared against with `memcmp` at `sv_send.c:1997` and
    /// `:2052`.
    static nullentitystate: EntityState;
    /// `Quake/gl_rmisc.c` -- johnfitz's per-frame developer counters.
    static mut dev_stats: DevStats;
    /// `Quake/gl_rmisc.c` -- the running peaks.
    static mut dev_peakstats: DevStats;
    /// `Quake/gl_rmisc.c` -- last time each overflow warning was printed.
    static mut dev_overflows: OverflowTimes;
}

// ---------------------------------------------------------------------------
// engine constants (protocol.h / quakedef.h / server.h / glquake.h)

/// `glquake.h:622` -- seconds between repeated overflow warnings.
const CONSOLE_RESPAM_TIME: f64 = 3.0;

/// `protocol.h:60`
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
/// `protocol.h:61`
const PEXT2_PREDINFO: c_uint = 0x0000_0020;

/// `protocol.h:46`
const PRFL_24BITCOORD: c_uint = 1 << 3;
/// `protocol.h:47`
const PRFL_FLOATCOORD: c_uint = 1 << 4;
/// `protocol.h:50`
const PRFL_INT32COORD: c_uint = 1 << 7;

/// `protocol.h:35`
const PROTOCOL_NETQUAKE: c_uint = 15;
/// `protocol.h:37`
const PROTOCOL_RMQ: c_uint = 999;

/// `protocol.h:102-138` -- the FTE replacement-delta bit set.
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
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_SOLID: c_uint = 1 << 13;
const UF_FLAGS: c_uint = 1 << 14;
const UF_EXTEND2: c_uint = 1 << 15;
const UF_ALPHA: c_uint = 1 << 16;
const UF_SCALE: c_uint = 1 << 17;
const UF_BONEDATA: c_uint = 1 << 18;
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_DRAWFLAGS: c_uint = 1 << 19;
const UF_TAGINFO: c_uint = 1 << 20;
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_LIGHT: c_uint = 1 << 21;
const UF_TRAILEFFECT: c_uint = 1 << 22;
const UF_EXTEND3: c_uint = 1 << 23;
const UF_COLORMOD: c_uint = 1 << 24;
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_GLOW: c_uint = 1 << 25;
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_FATNESS: c_uint = 1 << 26;
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_MODELINDEX2: c_uint = 1 << 27;
#[allow(dead_code)] // protocol constant, kept for the full table
const UF_GRAVITYDIR: c_uint = 1 << 28;
const UF_EFFECTS2: c_uint = 1 << 29;
const UF_UNUSED2: c_uint = 1 << 30;

/// `sv_send.c:146-151` -- server-side-only flags that re-use encoding bits.
const UF_REMOVE: c_uint = UF_16BIT;
const UF_MOVETYPE: c_uint = UF_EFFECTS2;
const UF_RESET2: c_uint = UF_EXTEND1;
const UF_WEAPONFRAME_OLD: c_uint = UF_EXTEND2;
const UF_VIEWANGLES: c_uint = UF_EXTEND3;

/// `protocol.h:141-149`
#[allow(dead_code)] // protocol constant, kept for the full table
const UFP_FORWARD: c_uint = 1 << 0;
#[allow(dead_code)] // protocol constant, kept for the full table
const UFP_SIDE: c_uint = 1 << 1;
#[allow(dead_code)] // protocol constant, kept for the full table
const UFP_UP: c_uint = 1 << 2;
const UFP_MOVETYPE: c_uint = 1 << 3;
const UFP_VELOCITYXY: c_uint = 1 << 4;
const UFP_VELOCITYZ: c_uint = 1 << 5;
#[allow(dead_code)] // protocol constant, kept for the full table
const UFP_MSEC: c_uint = 1 << 6;
/// `protocol.h:148`. "no longer used. just a stat now that I rewrote stat
/// deltas." -- it aliases [`UFP_VIEWANGLE`] deliberately.
const UFP_WEAPONFRAME_OLD: c_uint = 1 << 7;
const UFP_VIEWANGLE: c_uint = 1 << 7;

/// `protocol.h:67-93` -- the classic entity update bit set.
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
const U_ALPHA: c_int = 1 << 16;
const U_FRAME2: c_int = 1 << 17;
const U_MODEL2: c_int = 1 << 18;
const U_LERPFINISH: c_int = 1 << 19;
const U_SCALE: c_int = 1 << 20;
const U_EXTEND2: c_int = 1 << 23;

/// `protocol.h:152-184` -- the `svc_clientdata` bit set.
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
#[allow(dead_code)] // protocol constant, kept for the full table
const SU_EXTEND3: c_int = 1 << 31;
/// `protocol.h:187` -- DarkPlaces' viewzoom bit, reused here.
#[allow(dead_code)] // protocol constant, kept for the full table
const DPSU_VIEWZOOM: c_int = 1 << 19;

/// `protocol.h:218` / `:229` -- alpha and scale encoding defaults.
const ENTALPHA_DEFAULT: c_int = 0;
const ENTSCALE_DEFAULT: c_int = 16;

/// `quakedef.h` `stat_t` -- the stat slots `SV_CalcStats` fills.
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
const STAT_ITEMS: usize = 15;
const STAT_VIEWHEIGHT: usize = 16;
const STAT_VIEWZOOM: usize = 21;
const STAT_IDEALPITCH: usize = 25;
const STAT_PUNCHANGLE_X: usize = 26;
const STAT_PUNCHANGLE_Y: usize = 27;
const STAT_PUNCHANGLE_Z: usize = 28;

/// `progdefs.q1` -- `MOVETYPE_STEP`, compared against the `float` field.
const MOVETYPE_STEP: c_float = 4.0;
/// `progdefs.q1` -- `FL_ONGROUND`.
const FL_ONGROUND: c_int = 512;

/// `protocol.h:246-340` -- the svc opcodes emitted by this module.
const SVC_NOP: c_int = 1;
const SVC_UPDATESTAT: c_int = 3;
const SVC_TIME: c_int = 7;
const SVC_STUFFTEXT: c_int = 9;
const SVC_SETANGLE: c_int = 10;
const SVC_UPDATEFRAGS: c_int = 14;
const SVC_CLIENTDATA: c_int = 15;
const SVC_PARTICLE: c_int = 18;
const SVC_DAMAGE: c_int = 19;
const SVC_SPAWNSTATIC: c_int = 20;
const SVCFTE_SPAWNSTATIC2: c_int = 21;
const SVC_SPAWNBASELINE: c_int = 22;
const SVC_SIGNONNUM: c_int = 25;
const SVC_SPAWNSTATICSOUND: c_int = 29;
const SVC_SPAWNBASELINE2: c_int = 42;
const SVC_SPAWNSTATIC2: c_int = 43;
const SVC_SPAWNSTATICSOUND2: c_int = 44;
const SVCDP_UPDATESTATBYTE: c_int = 51;
const SVCDP_PRECACHE: c_int = 54;
const SVCFTE_SPAWNBASELINE2: c_int = 66;
const SVCFTE_UPDATESTATSTRING: c_int = 78;
const SVCFTE_UPDATESTATFLOAT: c_int = 79;
const SVCFTE_UPDATEENTITIES: c_int = 86;

/// `quakedef.h:93-95` -- precache table sizes.
#[allow(dead_code)] // protocol constant, kept for the full table
const MAX_MODELS: c_int = 8192;
#[allow(dead_code)] // protocol constant, kept for the full table
const MAX_SOUNDS: c_int = 2048;
const MAX_PARTICLETYPES: c_int = 2048;
/// `quakedef.h:109` -- stats below this index use the classic svc encodings.
#[allow(dead_code)] // protocol constant, kept for the full table
const MAX_CL_BASE_STATS: usize = 32;

/// How many pending `MSG_Write*` ops are batched into one guarded C call.
///
/// Purely a Rust-side buffering choice: the ops replay in insertion order
/// inside `SvSend_Glue_WriteBatch`, so the emitted byte stream is identical
/// for any batch size.
const WRITE_BATCH: usize = 64;

// ---------------------------------------------------------------------------
// File-scope state
//
// Every static below replaces a `static` in `sv_send.c`, i.e. an object that
// had internal linkage and therefore no C-visible name for `sv_send_glue.c`
// to own. The server runs single-threaded, exactly as the C did.
// ---------------------------------------------------------------------------

/// `sv_send.c:418-420` -- the shared snapshot scratch buffer.
static mut SNAPSHOT_ENTSTATE: *mut EntityNumState = ptr::null_mut();
static mut SNAPSHOT_NUMENTS: usize = 0;
static mut SNAPSHOT_MAXENTS: usize = 0;

/// `sv_send.c:1035-1038` -- `SV_FatPVS` accumulator.
static mut FATBYTES: c_int = 0;
static mut FATPVS: *mut u8 = ptr::null_mut();
static mut FATPVS_CAPACITY: c_int = 0;
static mut FATPVS_ANY: bool = false;

/// `sv_send.c:1129-1132` -- the `sv_netsort` radix scratch arrays.
static mut NET_EDICTS: [u16; MAX_EDICTS] = [0; MAX_EDICTS];
static mut NET_EDICT_DISTS: [u8; MAX_EDICTS] = [0; MAX_EDICTS];
static mut NET_EDICT_BINS: [c_int; 256] = [0; 256];
static mut NET_EDICTS_SORTED: [u16; MAX_EDICTS] = [0; MAX_EDICTS];

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// The ambient qcvm (ADR-008). `sv_send.c` only ever runs with the server
/// VM selected.
#[inline]
unsafe fn vm() -> *mut QcVm {
    // SAFETY: `qcvm` is the C-owned ambient VM pointer; the server is
    // single-threaded and `sv_send.c` reads it through the same macro.
    unsafe { c::qcvm.cast::<QcVm>() }
}

/// `qcvm->globals` viewed as `globalvars_t`, i.e. `pr_global_struct`.
#[inline]
unsafe fn globals() -> *mut GlobalVars {
    // SAFETY: `pr_global_struct` is the engine's own single-threaded pointer
    // to the progs global block. Read through the variable `sv_send.c:62` and
    // `:1571` name rather than through `qcvm->globals`: `progs.h:371` calls
    // the two "the same", but only the former is what the C source spells,
    // and they are separately assigned.
    unsafe { m::pr_global_struct.cast::<GlobalVars>() }
}

/// `progs.h` `NEXT_EDICT (e)`
#[inline]
unsafe fn next_edict(vm: *mut QcVm, e: *mut Edict) -> *mut Edict {
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro.
    unsafe {
        e.cast::<u8>()
            .wrapping_offset((*vm).edict_size as isize)
            .cast::<Edict>()
    }
}

/// `progs.h` `PROG_TO_EDICT (e)`
#[inline]
unsafe fn prog_to_edict(vm: *mut QcVm, p: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only; the C macro has no bounds check either.
    unsafe {
        (*vm)
            .edicts
            .cast::<u8>()
            .wrapping_offset(p as isize)
            .cast::<Edict>()
    }
}

/// `progs.h` `EDICT_TO_PROG (e)`
#[inline]
unsafe fn edict_to_prog(vm: *mut QcVm, e: *mut Edict) -> c_int {
    // SAFETY: pointer arithmetic only, byte-for-byte the C macro.
    unsafe { e.cast::<u8>().offset_from((*vm).edicts.cast::<u8>()) as c_int }
}

/// `progs.h` `EDICT_NUM (n)`, inlined as `EDICT_NUM_NO_CHECK`.
///
/// `EDICT_NUM` (`pr_edict.c:1156`) raises only for `n < 0 || n >=
/// qcvm->max_edicts`. Every `sv_send.c` call site indexes a loop bounded by
/// `qcvm->num_edicts`, which is `<= max_edicts` by construction, so the
/// check can never fire and no ADR-009 guard is needed here.
#[inline]
unsafe fn edict_num(vm: *mut QcVm, n: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only.
    unsafe {
        (*vm)
            .edicts
            .cast::<u8>()
            .wrapping_offset((n as isize) * ((*vm).edict_size as isize))
            .cast::<Edict>()
    }
}

/// `cvar_t::value`.
#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: `var` points at a `cvar_t` static owned by C; cvars are
    // single-threaded engine state.
    unsafe { ptr::addr_of!((*var).value).read() }
}

/// `PR_GetString (handle)` behind its ADR-009 guard.
#[inline]
unsafe fn get_string(handle: c_int, out: &mut *const c_char) -> Raise {
    // SAFETY: `out` is a live local; the guard writes it only on success.
    unsafe { g::SvSend_Glue_GetString(handle, out) }
}

/// `NUM_FOR_EDICT (e)` behind its ADR-009 guard.
#[inline]
unsafe fn num_for_edict(e: *mut Edict, out: &mut c_int) -> Raise {
    // SAFETY: `out` is a live local; the guard writes it only on success.
    unsafe { w::World_Glue_NumForEdict(e.cast::<c_void>(), out) }
}

// ---------------------------------------------------------------------------
// eval_t accessors
//
// `quake_c_sys::sv_phys::GetEdictFieldValue` returns `*mut c_float` (the
// first member of the C union). Every other member is read by re-casting
// that pointer. `progs.h:29-41` gives the union no alignment above 4, and
// the 64-bit members are only 4-aligned inside the entity field block, so
// those go through `read_unaligned`.
// ---------------------------------------------------------------------------

#[inline]
unsafe fn ev_float(p: *mut c_float) -> c_float {
    // SAFETY: `p` is a live `eval_t *` from `GetEdictFieldValue`.
    unsafe { p.read() }
}

#[inline]
unsafe fn ev_int(p: *mut c_float) -> i32 {
    // SAFETY: `eval_t::_int` overlays `_float`; same address, same size.
    unsafe { p.cast::<i32>().read() }
}

#[inline]
unsafe fn ev_uint32(p: *mut c_float) -> u32 {
    // SAFETY: `eval_t::_uint32` overlays `_float`.
    unsafe { p.cast::<u32>().read() }
}

#[inline]
unsafe fn ev_sint64(p: *mut c_float) -> i64 {
    // SAFETY: `eval_t::_sint64` overlays the union base; entity fields are
    // only 4-byte aligned, hence the unaligned read.
    unsafe { p.cast::<i64>().read_unaligned() }
}

#[inline]
unsafe fn ev_uint64(p: *mut c_float) -> u64 {
    // SAFETY: as `ev_sint64`.
    unsafe { p.cast::<u64>().read_unaligned() }
}

#[inline]
unsafe fn ev_double(p: *mut c_float) -> f64 {
    // SAFETY: as `ev_sint64`.
    unsafe { p.cast::<f64>().read_unaligned() }
}

#[inline]
unsafe fn ev_vector(p: *mut c_float, i: usize) -> c_float {
    // SAFETY: `eval_t::vector` is `float[3]` at the union base.
    unsafe { p.add(i).read() }
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

/// `q_minmax.h:70` `Q_rint (x)` for a `float` argument.
///
/// COMPAT: ADR-010. The literal `0.5` is a `double`, so the C promotes the
/// `float` operand and rounds in double precision. Widening here is
/// therefore required, not a convenience.
#[inline]
fn q_rint_f(x: c_float) -> c_int {
    if x > 0.0 {
        (x as f64 + 0.5) as c_int
    } else {
        (x as f64 - 0.5) as c_int
    }
}

/// `q_minmax.h:70` `Q_rint (x)` for a `double` argument -- the literal
/// `0.5` is already `double`, so nothing is promoted.
#[inline]
fn q_rint_d(x: f64) -> c_int {
    if x > 0.0 {
        (x + 0.5) as c_int
    } else {
        (x - 0.5) as c_int
    }
}

/// `q_minmax.h:38` `clamp_f`.
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

/// `q_min` (`q_minmax.h:60`) on two `float`s. NaN-ordering matters, so this
/// is the C conditional, not `f32::min`.
#[inline]
fn min_f(a: c_float, b: c_float) -> c_float {
    if a < b {
        a
    } else {
        b
    }
}

/// `q_max` (`q_minmax.h:65`) on two `float`s.
#[inline]
fn max_f(a: c_float, b: c_float) -> c_float {
    if a > b {
        a
    } else {
        b
    }
}

/// `q_minmax.h:26` `clamp_i`.
#[inline]
fn clamp_i(minval: c_int, val: c_int, maxval: c_int) -> c_int {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `protocol.h:222` `ENTALPHA_ENCODE (a)`.
///
/// COMPAT: ADR-010. `CLAMP (1, (a) * 254.0f + 1, 255)`'s `_Generic`
/// selector is `int + float + int`, i.e. `float`, so the clamp runs in f32;
/// `Q_rint` then promotes the clamped f32 to double.
#[inline]
fn entalpha_encode(a: c_float) -> c_int {
    if a == 0.0 {
        ENTALPHA_DEFAULT
    } else {
        q_rint_f(clamp_f(1.0, a * 254.0 + 1.0, 255.0))
    }
}

/// `protocol.h:230` `ENTSCALE_ENCODE (f)`.
///
/// COMPAT: ADR-010. `ENTSCALE_DEFAULT * (f)` is `int * float`, i.e. f32,
/// truncated to `int` before a `clamp_i`.
#[inline]
fn entscale_encode(f: c_float) -> c_int {
    if f != 0.0 {
        clamp_i(1, (ENTSCALE_DEFAULT as c_float * f) as c_int, 255)
    } else {
        ENTSCALE_DEFAULT
    }
}

// ---------------------------------------------------------------------------
// Buffered writer
// ---------------------------------------------------------------------------

/// `svsend_write_t.kind` values -- must match `Quake/sv_send_glue.c`.
const W_BYTE: c_int = 0;
const W_CHAR: c_int = 1;
const W_SHORT: c_int = 2;
const W_LONG: c_int = 3;
const W_FLOAT: c_int = 4;
const W_STRING: c_int = 5;
const W_COORD: c_int = 6;
const W_ANGLE: c_int = 7;
const W_ANGLE16: c_int = 8;
const W_ENTITY: c_int = 9;
const W_SZWRITE: c_int = 10;

/// Accumulates `MSG_Write*`/`SZ_Write` calls against one `sizebuf_t` and
/// hands them to `SvSend_Glue_WriteBatch`, which replays the run inside a
/// single `Host_Guard` frame (ADR-009 rule 3).
///
/// Unlike `sv_main.c`, `sv_send.c` reads `msg->cursize` *between* writes --
/// packet budgets, overflow tests and rollback points all do. Every field
/// accessor below therefore flushes first, which makes a stale read
/// structurally impossible rather than merely unlikely.
struct Writer {
    sb: *mut SizeBuf,
    ops: [g::SvSendWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new(sb: *mut SizeBuf) -> Self {
        Writer {
            sb,
            ops: [g::SvSendWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                u: 0,
                p: ptr::null(),
            }; WRITE_BATCH],
            n: 0,
        }
    }

    unsafe fn flush(&mut self) -> Raise {
        if self.n == 0 {
            return 0;
        }
        let count = self.n;
        self.n = 0;
        // SAFETY: `sb` points at a live `sizebuf_t`; `ops[..count]` is
        // initialised and every `p` pointer is still live at this point.
        unsafe {
            g::SvSend_Glue_WriteBatch(self.sb.cast::<c_void>(), self.ops.as_ptr(), count as c_int)
        }
    }

    unsafe fn push(
        &mut self,
        kind: c_int,
        i: c_int,
        f: c_float,
        u: c_uint,
        p: *const c_void,
    ) -> Raise {
        if self.n == WRITE_BATCH {
            // SAFETY: see `flush`.
            let r = unsafe { self.flush() };
            if r != 0 {
                return r;
            }
        }
        self.ops[self.n] = g::SvSendWriteOp { kind, i, f, u, p };
        self.n += 1;
        0
    }

    unsafe fn byte(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_BYTE, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn char_(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_CHAR, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn short(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_SHORT, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn long(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_LONG, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn float(&mut self, v: c_float) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_FLOAT, 0, v, 0, ptr::null()) }
    }

    /// `s` must stay live until the next flush.
    unsafe fn string(&mut self, s: *const c_char) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_STRING, 0, 0.0, 0, s.cast::<c_void>()) }
    }

    unsafe fn coord(&mut self, f: c_float, flags: c_uint) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_COORD, 0, f, flags, ptr::null()) }
    }

    unsafe fn angle(&mut self, f: c_float, flags: c_uint) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_ANGLE, 0, f, flags, ptr::null()) }
    }

    unsafe fn angle16(&mut self, f: c_float, flags: c_uint) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_ANGLE16, 0, f, flags, ptr::null()) }
    }

    unsafe fn entity(&mut self, num: c_int, pext2: c_uint) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_ENTITY, num, 0.0, pext2, ptr::null()) }
    }

    /// `data` must stay live until the next flush.
    unsafe fn sz_write(&mut self, data: *const c_void, len: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_SZWRITE, len, 0.0, 0, data) }
    }

    /// `msg->cursize`, with all pending ops applied first.
    unsafe fn cursize(&mut self, out: &mut c_int) -> Raise {
        // SAFETY: `sb` is live; the read follows a successful flush.
        unsafe {
            raise!(self.flush());
            *out = (*self.sb).cursize;
        }
        0
    }

    /// `msg->cursize = v`, with all pending ops applied first.
    unsafe fn set_cursize(&mut self, v: c_int) -> Raise {
        // SAFETY: as `cursize`.
        unsafe {
            raise!(self.flush());
            (*self.sb).cursize = v;
        }
        0
    }

    /// `msg->maxsize`, with all pending ops applied first.
    unsafe fn maxsize(&mut self, out: &mut c_int) -> Raise {
        // SAFETY: as `cursize`.
        unsafe {
            raise!(self.flush());
            *out = (*self.sb).maxsize;
        }
        0
    }

    /// `msg->maxsize = v`, with all pending ops applied first.
    unsafe fn set_maxsize(&mut self, v: c_int) -> Raise {
        // SAFETY: as `cursize`.
        unsafe {
            raise!(self.flush());
            (*self.sb).maxsize = v;
        }
        0
    }

    /// `msg->overflowed`, with all pending ops applied first.
    unsafe fn overflowed(&mut self, out: &mut bool) -> Raise {
        // SAFETY: as `cursize`.
        unsafe {
            raise!(self.flush());
            *out = (*self.sb).overflowed;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:34 SV_UsePredThinkPos

/// `sv_send.c:34`. `static` in C.
///
/// COMPAT: ADR-010. `qcvm->time` is `double` and `ent->lastthink` is
/// `float`, so `qcvm->time - ent->lastthink` computes in double and is then
/// *narrowed* to the `float elapsedtime`. The comparison `elapsedtime > 0.1`
/// promotes that f32 back to double against the double literal `0.1`.
unsafe fn sv_use_pred_think_pos(ent: *mut Edict) -> bool {
    // SAFETY: `ent` is a live edict; the cvars and `isDedicated` are
    // single-threaded engine state.
    unsafe {
        if cvar_value(ptr::addr_of!(m::sv_smoothplatformlerps)) == 0.0
            || (!c::isDedicated && cvar_value(ptr::addr_of!(g::r_lerpmove)) == 0.0)
        {
            return false;
        }
        if (*ent).v.movetype != MOVETYPE_STEP {
            return false;
        }
        if ((*ent).v.flags as c_int & FL_ONGROUND) == 0 {
            return false;
        }
        let elapsedtime: c_float = ((*vm()).time - (*ent).lastthink as f64) as c_float;
        if elapsedtime < 0.0 || elapsedtime as f64 > 0.1 {
            return false;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:51 SV_CalcStats

/// `sv_send.c:51` `SV_CalcStats`.
///
/// Statusized (ADR-009): `PR_GetString` and `NUM_FOR_EDICT` both raise.
unsafe fn sv_calc_stats(
    client: *mut Client,
    statsi: *mut c_int,
    statsf: *mut c_float,
    statss: *mut *const c_char,
) -> Raise {
    // SAFETY: `client` is a live `client_t` with a live `edict`; the three
    // stat arrays are caller-provided `MAX_CL_STATS`-element buffers.
    unsafe {
        let vm = vm();
        let ent = (*client).edict;
        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.items2);
        let items: c_int = if !val.is_null() {
            // COMPAT: ADR-010 rule 8 -- C's float-to-unsigned conversion is
            // undefined out of range; Rust's `as` saturates.
            ((*ent).v.items as u32 | ((ev_float(val) as u32) << 23)) as c_int
        } else {
            ((*ent).v.items as u32 | (((*globals()).serverflags as u32) << 28)) as c_int
        };

        ptr::write_bytes(statsi, 0, MAX_CL_STATS);
        ptr::write_bytes(statsf, 0, MAX_CL_STATS);
        ptr::write_bytes(statss, 0, MAX_CL_STATS);

        *statsf.add(STAT_HEALTH) = (*ent).v.health;

        let mut weaponmodel: *const c_char = ptr::null();
        raise!(get_string((*ent).v.weaponmodel, &mut weaponmodel));
        *statsi.add(STAT_WEAPON) = g::SvSend_Glue_ModelIndex(weaponmodel);
        if (*statsi.add(STAT_WEAPON)) as c_uint >= (*client).limit_models {
            *statsi.add(STAT_WEAPON) = 0;
        }
        *statsf.add(STAT_AMMO) = (*ent).v.currentammo;
        *statsf.add(STAT_ARMOR) = (*ent).v.armorvalue;
        *statsf.add(STAT_WEAPONFRAME) = (*ent).v.weaponframe;
        *statsf.add(STAT_SHELLS) = (*ent).v.ammo_shells;
        *statsf.add(STAT_NAILS) = (*ent).v.ammo_nails;
        *statsf.add(STAT_ROCKETS) = (*ent).v.ammo_rockets;
        *statsf.add(STAT_CELLS) = (*ent).v.ammo_cells;
        *statsf.add(STAT_ACTIVEWEAPON) = (*ent).v.weapon;

        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.viewzoom);
        if !val.is_null() && ev_float(val) != 0.0 {
            // COMPAT: ADR-010. `val->_float * 255` is `float * int`, i.e. f32.
            *statsf.add(STAT_VIEWZOOM) = ev_float(val) * 255.0;
            if *statsf.add(STAT_VIEWZOOM) < 1.0 {
                *statsf.add(STAT_VIEWZOOM) = 1.0;
            }
        }

        if ((*client).protocol_pext2 & PEXT2_PREDINFO) != 0 {
            *statsi.add(STAT_ITEMS) = items;
            *statsf.add(STAT_VIEWHEIGHT) = (*ent).v.view_ofs[2];
            *statsf.add(STAT_IDEALPITCH) = (*ent).v.idealpitch;
            *statsf.add(STAT_PUNCHANGLE_X) = (*ent).v.punchangle[0];
            *statsf.add(STAT_PUNCHANGLE_Y) = (*ent).v.punchangle[1];
            *statsf.add(STAT_PUNCHANGLE_Z) = (*ent).v.punchangle[2];
        }

        for i in 0..sv.numcustomstats {
            let cs = ptr::addr_of!(sv.customstats[i]);
            let mut eval = (*cs).ptr.cast::<c_float>();
            if eval.is_null() {
                eval = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*cs).fld);
            }
            let idx = (*cs).idx as usize;

            match (*cs).r#type {
                etype::EV_EXT_INTEGER => *statsi.add(idx) = ev_int(eval),
                etype::EV_EXT_UINT32 => *statsi.add(idx) = ev_uint32(eval) as c_int,
                etype::EV_EXT_SINT64 => {
                    let v = ev_sint64(eval);
                    *statsi.add(idx) = v as c_int;
                    *statsi.add(idx + 1) = (v >> 32) as c_int;
                }
                etype::EV_EXT_UINT64 => {
                    let v = ev_uint64(eval);
                    *statsi.add(idx) = v as c_int;
                    *statsi.add(idx + 1) = (v >> 32) as c_int;
                }
                // COMPAT: ADR-010. The C narrows `double` to `float` here
                // ("FIXME: precision loss" in the original); preserved.
                etype::EV_EXT_DOUBLE => *statsf.add(idx) = ev_double(eval) as c_float,
                etype::EV_ENTITY => {
                    let e = prog_to_edict(vm, ev_int(eval));
                    let mut num: c_int = 0;
                    raise!(num_for_edict(e, &mut num));
                    *statsi.add(idx) = num;
                }
                etype::EV_FLOAT => *statsf.add(idx) = ev_float(eval),
                etype::EV_VECTOR => {
                    *statsf.add(idx) = ev_vector(eval, 0);
                    *statsf.add(idx + 1) = ev_vector(eval, 1);
                    *statsf.add(idx + 2) = ev_vector(eval, 2);
                }
                etype::EV_STRING => {
                    let mut s: *const c_char = ptr::null();
                    raise!(get_string(ev_int(eval), &mut s));
                    *statss.add(idx) = s;
                }
                _ => {}
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:152 SVFTE_DeltaPredCalcBits

/// `sv_send.c:152`. `from` is unused in the shipped body (every
/// `from`-dependent test is commented out upstream); kept in the signature
/// so the two call sites read the same as the C.
fn svfte_delta_pred_calc_bits(_from: *const EntityState, to: *const EntityState) -> c_uint {
    let mut bits: c_uint = 0;
    // SAFETY: `to` is a live `entity_state_t`.
    unsafe {
        if (*to).velocity[0] != 0 {
            bits |= UFP_VELOCITYXY;
        }
        if (*to).velocity[1] != 0 {
            bits |= UFP_VELOCITYXY;
        }
        if (*to).velocity[2] != 0 {
            bits |= UFP_VELOCITYZ;
        }
    }
    bits
}

// ---------------------------------------------------------------------------
// sv_send.c:175 MSGFTE_DeltaCalcBits

/// `sv_send.c:175`. Pure comparison; cannot raise.
unsafe fn msgfte_delta_calc_bits(from: *const EntityState, to: *const EntityState) -> c_uint {
    let mut bits: c_uint = 0;

    // SAFETY: both pointers are live `entity_state_t`s.
    unsafe {
        if (*from).pmovetype != (*to).pmovetype {
            bits |= UF_PREDINFO | UF_MOVETYPE;
        }
        {
            if svfte_delta_pred_calc_bits(from, to) != 0 {
                bits |= UF_PREDINFO;
            }

            if (bits & UF_PREDINFO) != 0
                && ((*from).velocity[0] != 0
                    || (*from).velocity[1] != 0
                    || (*from).velocity[2] != 0)
            {
                bits |= UF_ORIGINXY | UF_ORIGINZ;
            }
        }

        if (*to).origin[0] != (*from).origin[0] {
            bits |= UF_ORIGINXY;
        }
        if (*to).origin[1] != (*from).origin[1] {
            bits |= UF_ORIGINXY;
        }
        if (*to).origin[2] != (*from).origin[2] {
            bits |= UF_ORIGINZ;
        }

        if (*to).angles[0] != (*from).angles[0] {
            bits |= UF_ANGLESXZ;
        }
        if (*to).angles[1] != (*from).angles[1] {
            bits |= UF_ANGLESY;
        }
        if (*to).angles[2] != (*from).angles[2] {
            bits |= UF_ANGLESXZ;
        }

        if (*to).modelindex != (*from).modelindex {
            bits |= UF_MODEL;
        }
        if (*to).frame != (*from).frame {
            bits |= UF_FRAME;
        }
        if (*to).skin != (*from).skin {
            bits |= UF_SKIN;
        }
        if (*to).colormap != (*from).colormap {
            bits |= UF_COLORMAP;
        }
        if (*to).effects != (*from).effects {
            bits |= UF_EFFECTS;
        }
        if (*to).eflags != (*from).eflags {
            bits |= UF_FLAGS;
        }
        if (*to).scale != (*from).scale {
            bits |= UF_SCALE;
        }
        if (*to).alpha != (*from).alpha {
            bits |= UF_ALPHA;
        }
        if (*to).colormod[0] != (*from).colormod[0]
            || (*to).colormod[1] != (*from).colormod[1]
            || (*to).colormod[2] != (*from).colormod[2]
        {
            bits |= UF_COLORMOD;
        }
        if (*to).tagentity != (*from).tagentity || (*to).tagindex != (*from).tagindex {
            bits |= UF_TAGINFO;
        }
        if (*to).traileffectnum != (*from).traileffectnum
            || (*to).emiteffectnum != (*from).emiteffectnum
        {
            bits |= UF_TRAILEFFECT;
        }
        // `LERP_BANDAID` is unconditionally defined (`protocol.h:33`).
        if (*to).lerp != (*from).lerp {
            bits |= UF_UNUSED2;
        }
    }

    bits
}

// ---------------------------------------------------------------------------
// sv_send.c:239 MSGFTE_WriteEntityUpdate

/// `sv_send.c:239`. Statusized: every write goes through the batching
/// guard.
unsafe fn msgfte_write_entity_update(
    wr: &mut Writer,
    mut bits: c_uint,
    state: *const EntityState,
    pext2: c_uint,
    protocolflags: c_uint,
) -> Raise {
    // SAFETY: `state` is a live `entity_state_t`; `wr` wraps a live sizebuf.
    unsafe {
        let mut predbits: c_uint = 0;
        if (bits & UF_MOVETYPE) != 0 {
            bits &= !UF_MOVETYPE;
            predbits |= UFP_MOVETYPE;
        }
        if (pext2 & PEXT2_PREDINFO) != 0 {
            if (bits & UF_VIEWANGLES) != 0 {
                bits &= !UF_VIEWANGLES;
                bits |= UF_PREDINFO;
                predbits |= UFP_VIEWANGLE;
            }
        } else {
            if (bits & UF_VIEWANGLES) != 0 {
                bits &= !UF_VIEWANGLES;
                bits |= UF_PREDINFO;
            }
            if (bits & UF_WEAPONFRAME_OLD) != 0 {
                bits &= !UF_WEAPONFRAME_OLD;
                predbits |= UFP_WEAPONFRAME_OLD;
            }
        }

        // `LERP_BANDAID` (`protocol.h:33`). The `cls.demorecording ||
        // strcmp (..., "LOCAL")` test stays in C -- see
        // `SvSend_Glue_StripLerp`.
        if (bits & UF_UNUSED2) != 0 && g::SvSend_Glue_StripLerp() != 0 {
            bits &= !UF_UNUSED2;
        }

        bits &= !UF_BONEDATA;

        if (bits & UF_MODEL) != 0 && (*state).modelindex > 255 {
            bits |= UF_16BIT;
        }
        if (bits & UF_FRAME) != 0 && (*state).frame > 255 {
            bits |= UF_16BIT;
        }

        if (bits & UF_EFFECTS) != 0 {
            if ((*state).effects & 0xffff_0000) != 0 {
                bits |= UF_EFFECTS | UF_EFFECTS2;
            } else if ((*state).effects & 0x0000_ff00) != 0 {
                bits = (bits & !UF_EFFECTS) | UF_EFFECTS2;
            }
        }
        if (bits & 0xff00_0000) != 0 {
            bits |= UF_EXTEND3;
        }
        if (bits & 0x00ff_0000) != 0 {
            bits |= UF_EXTEND2;
        }
        if (bits & 0x0000_ff00) != 0 {
            bits |= UF_EXTEND1;
        }

        #[allow(clippy::identity_op)] // the C spells the first byte `bits >> 0`
        {
            raise!(wr.byte(((bits >> 0) & 0xff) as c_int));
        }
        if (bits & UF_EXTEND1) != 0 {
            raise!(wr.byte(((bits >> 8) & 0xff) as c_int));
        }
        if (bits & UF_EXTEND2) != 0 {
            raise!(wr.byte(((bits >> 16) & 0xff) as c_int));
        }
        if (bits & UF_EXTEND3) != 0 {
            raise!(wr.byte(((bits >> 24) & 0xff) as c_int));
        }

        if (bits & UF_FRAME) != 0 {
            if (bits & UF_16BIT) != 0 {
                raise!(wr.short((*state).frame as c_int));
            } else {
                raise!(wr.byte((*state).frame as c_int));
            }
        }
        if (bits & UF_ORIGINXY) != 0 {
            raise!(wr.coord((*state).origin[0], protocolflags));
            raise!(wr.coord((*state).origin[1], protocolflags));
        }
        if (bits & UF_ORIGINZ) != 0 {
            raise!(wr.coord((*state).origin[2], protocolflags));
        }

        if (bits & UF_PREDINFO) != 0 && (pext2 & PEXT2_PREDINFO) == 0 {
            if (bits & UF_ANGLESXZ) != 0 {
                raise!(wr.angle16((*state).angles[0], protocolflags));
                raise!(wr.angle16((*state).angles[2], protocolflags));
            }
            if (bits & UF_ANGLESY) != 0 {
                raise!(wr.angle16((*state).angles[1], protocolflags));
            }
        } else {
            if (bits & UF_ANGLESXZ) != 0 {
                raise!(wr.angle((*state).angles[0], protocolflags));
                raise!(wr.angle((*state).angles[2], protocolflags));
            }
            if (bits & UF_ANGLESY) != 0 {
                raise!(wr.angle((*state).angles[1], protocolflags));
            }
        }

        if (bits & (UF_EFFECTS | UF_EFFECTS2)) == (UF_EFFECTS | UF_EFFECTS2) {
            raise!(wr.long((*state).effects as c_int));
        } else if (bits & UF_EFFECTS2) != 0 {
            raise!(wr.short((*state).effects as c_int));
        } else if (bits & UF_EFFECTS) != 0 {
            raise!(wr.byte((*state).effects as c_int));
        }

        if (bits & UF_PREDINFO) != 0 {
            predbits |= svfte_delta_pred_calc_bits(ptr::null(), state);

            raise!(wr.byte(predbits as c_int));
            if (predbits & UFP_MOVETYPE) != 0 {
                raise!(wr.byte((*state).pmovetype as c_int));
            }
            if (predbits & UFP_VELOCITYXY) != 0 {
                raise!(wr.short((*state).velocity[0] as c_int));
                raise!(wr.short((*state).velocity[1] as c_int));
            }
            if (predbits & UFP_VELOCITYZ) != 0 {
                raise!(wr.short((*state).velocity[2] as c_int));
            }
        }

        if (bits & UF_MODEL) != 0 {
            if (bits & UF_16BIT) != 0 {
                raise!(wr.short((*state).modelindex as c_int));
            } else {
                raise!(wr.byte((*state).modelindex as c_int));
            }
        }
        if (bits & UF_SKIN) != 0 {
            if (bits & UF_16BIT) != 0 {
                raise!(wr.short((*state).skin as c_int));
            } else {
                raise!(wr.byte((*state).skin as c_int));
            }
        }
        if (bits & UF_COLORMAP) != 0 {
            raise!(wr.byte((*state).colormap as c_int & 0xff));
        }
        if (bits & UF_FLAGS) != 0 {
            raise!(wr.byte((*state).eflags as c_int));
        }

        if (bits & UF_ALPHA) != 0 {
            raise!(wr.byte(((*state).alpha as c_int - 1) & 0xff));
        }
        if (bits & UF_SCALE) != 0 {
            raise!(wr.byte((*state).scale as c_int));
        }

        if (bits & UF_TAGINFO) != 0 {
            raise!(wr.entity((*state).tagentity as c_int, pext2));
            raise!(wr.byte((*state).tagindex as c_int));
        }

        if (bits & UF_TRAILEFFECT) != 0 {
            if (*state).emiteffectnum != 0 {
                raise!(wr.short((((*state).traileffectnum & 0x3fff) | 0x8000) as c_int));
                raise!(wr.short(((*state).emiteffectnum & 0x3fff) as c_int));
            } else {
                raise!(wr.short(((*state).traileffectnum & 0x3fff) as c_int));
            }
        }

        if (bits & UF_COLORMOD) != 0 {
            raise!(wr.byte((*state).colormod[0] as c_int));
            raise!(wr.byte((*state).colormod[1] as c_int));
            raise!(wr.byte((*state).colormod[2] as c_int));
        }

        // `LERP_BANDAID`
        if (bits & UF_UNUSED2) != 0 {
            raise!(wr.short((*state).lerp as c_int));
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:423 SVFTE_DestroyFrames

/// `sv_send.c:423`. Frees only; cannot raise.
unsafe fn svfte_destroy_frames(client: *mut Client) {
    // SAFETY: `client` is a live `client_t`; every pointer freed here was
    // allocated by `SVFTE_SetupFrames`/`SVFTE_CalcEntityDeltas`.
    unsafe {
        for i in 0..MAX_CL_STATS {
            if (*client).oldstats_s[i].is_null() {
                continue;
            }
            c::Mem_Free((*client).oldstats_s[i].cast::<c_void>());
            (*client).oldstats_s[i] = ptr::null_mut();
        }
        if !(*client).previousentities.is_null() {
            c::Mem_Free((*client).previousentities.cast::<c_void>());
        }
        (*client).previousentities = ptr::null_mut();
        (*client).numpreviousentities = 0;
        (*client).maxpreviousentities = 0;

        if !(*client).pendingentities_bits.is_null() {
            c::Mem_Free((*client).pendingentities_bits.cast::<c_void>());
        }
        (*client).pendingentities_bits = ptr::null_mut();
        (*client).numpendingentities = 0;

        while (*client).numframes > 0 {
            (*client).numframes -= 1;
            c::Mem_Free(
                (*(*client).frames.add((*client).numframes))
                    .ents
                    .cast::<c_void>(),
            );
        }
        if !(*client).frames.is_null() {
            c::Mem_Free((*client).frames.cast::<c_void>());
        }
        (*client).frames = ptr::null_mut();

        (*client).lastacksequence = 0;
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:455 SVFTE_SetupFrames

/// `sv_send.c:455`. `Mem_Alloc` aborts rather than raising, so no status.
unsafe fn svfte_setup_frames(client: *mut Client) {
    // SAFETY: `client` is a live `client_t`; the ambient qcvm is the server's.
    unsafe {
        ptr::write_bytes((*client).oldstats_i.as_mut_ptr(), 0, MAX_CL_STATS);
        ptr::write_bytes((*client).oldstats_f.as_mut_ptr(), 0, MAX_CL_STATS);
        (*client).lastmovemessage = 0;

        if (*client).protocol_pext2 == 0 {
            svfte_destroy_frames(client);
            return;
        }

        (*client).numframes = 64;
        (*client).frames = c::Mem_Alloc(core::mem::size_of::<DeltaFrame>() * (*client).numframes)
            .cast::<DeltaFrame>();
        (*client).lastacksequence = 0x8000_0000u32 as c_int;
        ptr::write_bytes((*client).frames, 0, (*client).numframes);
        for fr in 0..(*client).numframes {
            (*(*client).frames.add(fr)).sequence = (*client).lastacksequence;
        }

        (*client).numpendingentities = (*vm()).num_edicts as usize;
        (*client).pendingentities_bits =
            c::Mem_Alloc((*client).numpendingentities * core::mem::size_of::<c_uint>())
                .cast::<c_uint>();

        *(*client).pendingentities_bits = UF_REMOVE;
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:482 SVFTE_DroppedFrame

/// `sv_send.c:482`. `static` in C. Cannot raise.
unsafe fn svfte_dropped_frame(client: *mut Client, sequence: c_int) {
    // SAFETY: `client` is a live `client_t` with `numframes` a power of two.
    unsafe {
        let frame = (*client)
            .frames
            .add((sequence & ((*client).numframes as c_int - 1)) as usize);
        if (*frame).sequence != sequence {
            return;
        }
        (*frame).sequence = -1;
        for i in 0..MAX_CL_STATS / 32 {
            (*client).resendstatsnum[i] |= (*frame).resendstatsnum[i];
            (*client).resendstatsstr[i] |= (*frame).resendstatsstr[i];
        }
        for i in 0..(*frame).numents {
            let ent = (*frame).ents.add(i as usize);
            if (*ent).ebits != 0 {
                *(*client).pendingentities_bits.add((*ent).num as usize) |= (*ent).ebits;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:503 SVFTE_Ack

/// `sv_send.c:503`. Cannot raise.
///
/// COMPAT: ADR-010. `qcvm->time - frame->timestamp` computes in double
/// (`qcvm->time` is `double`) and is narrowed on assignment to the `float`
/// `ping_times` slot.
unsafe fn svfte_ack(client: *mut Client, sequence: c_int) {
    // SAFETY: `client` is a live `client_t`; `host_client` is the client
    // currently being serviced.
    unsafe {
        let mut dropseq = (*client).lastacksequence + 1;
        if (*client).numframes == 0 {
            return;
        }
        if sequence == -1 {
            *(*client).pendingentities_bits |= UF_REMOVE;
        }
        if sequence < (*client).lastacksequence {
            return;
        }
        if (dropseq.wrapping_sub(sequence)) as c_uint >= (*client).numframes as c_uint {
            dropseq = sequence - (*client).numframes as c_int;
        }
        while dropseq < sequence {
            svfte_dropped_frame(client, dropseq);
            dropseq += 1;
        }
        (*client).lastacksequence = sequence;

        let frame = (*client)
            .frames
            .add((sequence & ((*client).numframes as c_int - 1)) as usize);
        if (*frame).sequence >= 0 {
            (*frame).sequence = -1;
            let idx = ((*host_client).num_pings as usize) % NUM_PING_TIMES;
            (*host_client).ping_times[idx] = ((*vm()).time - (*frame).timestamp as f64) as c_float;
            (*host_client).num_pings += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:532 SVFTE_WriteStats

/// `sv_send.c:532`. `static` in C. Statusized: `SV_CalcStats` and every
/// write can raise.
///
/// COMPAT: ADR-010. `statsi[i] = statsf[i]` is a float-to-int conversion,
/// undefined in C when out of range and saturating in Rust (rule 8). The
/// `(double)statsi[i] != statsf[i]` round-trip test promotes the `float`
/// to `double`, exactly as written.
unsafe fn svfte_write_stats(client: *mut Client, wr: &mut Writer) -> Raise {
    // SAFETY: `client` is a live `client_t`; the three stat arrays are
    // stack locals of `MAX_CL_STATS` elements each, and every `statss`
    // entry points into the engine string table, which outlives the flush.
    unsafe {
        let mut statsi: [c_int; MAX_CL_STATS] = [0; MAX_CL_STATS];
        let mut statsf: [c_float; MAX_CL_STATS] = [0.0; MAX_CL_STATS];
        let mut statss: [*const c_char; MAX_CL_STATS] = [ptr::null(); MAX_CL_STATS];
        let sequence = g::NET_QSocketGetSequenceOut((*client).netconnection.cast::<c_void>());

        let maxstats = if ((*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS) != 0 {
            MAX_CL_STATS
        } else {
            32
        };

        let frame = (*client)
            .frames
            .add((sequence & ((*client).numframes as c_int - 1)) as usize);

        if (*frame).sequence == sequence - (*client).numframes as c_int {
            svfte_dropped_frame(client, (*frame).sequence);
        }

        raise!(sv_calc_stats(
            client,
            statsi.as_mut_ptr(),
            statsf.as_mut_ptr(),
            statss.as_mut_ptr(),
        ));

        for i in 0..maxstats {
            if statsi[i] == 0 {
                statsi[i] = statsf[i] as c_int;
            } else {
                statsf[i] = 0.0;
            }

            if statsi[i] != (*client).oldstats_i[i] || statsf[i] != (*client).oldstats_f[i] {
                (*client).oldstats_i[i] = statsi[i];
                (*client).oldstats_f[i] = statsf[i];
                (*client).resendstatsnum[i / 32] |= 1u32 << (i & 31);
            }

            if !statss[i].is_null() || !(*client).oldstats_s[i].is_null() {
                let mut os: *const c_char = (*client).oldstats_s[i];
                let mut ns: *const c_char = statss[i];
                if ns.is_null() {
                    ns = c"".as_ptr();
                }
                if os.is_null() {
                    os = c"".as_ptr();
                }
                if g::strcmp(os, ns) != 0 {
                    (*client).resendstatsstr[i / 32] |= 1u32 << (i & 31);
                    c::Mem_Free((*client).oldstats_s[i].cast::<c_void>());
                    (*client).oldstats_s[i] = cc::q_strdup(ns);
                }
            }

            if ((*client).resendstatsnum[i / 32] & (1u32 << (i & 31))) != 0 {
                (*client).resendstatsnum[i / 32] &= !(1u32 << (i & 31));
                (*frame).resendstatsnum[i / 32] |= 1u32 << (i & 31);

                if statsi[i] as f64 != statsf[i] as f64 && statsf[i] != 0.0 {
                    raise!(wr.byte(SVCFTE_UPDATESTATFLOAT));
                    raise!(wr.byte(i as c_int));
                    raise!(wr.float(statsf[i]));
                } else if statsi[i] < 0 || statsi[i] > 255 {
                    raise!(wr.byte(SVC_UPDATESTAT));
                    raise!(wr.byte(i as c_int));
                    raise!(wr.long(statsi[i]));
                } else {
                    raise!(wr.byte(SVCDP_UPDATESTATBYTE));
                    raise!(wr.byte(i as c_int));
                    raise!(wr.byte(statsi[i]));
                }
            }

            if ((*client).resendstatsstr[i / 32] & (1u32 << (i & 31))) != 0 {
                (*client).resendstatsstr[i / 32] &= !(1u32 << (i & 31));
                (*frame).resendstatsstr[i / 32] |= 1u32 << (i & 31);

                raise!(wr.byte(SVCFTE_UPDATESTATSTRING));
                raise!(wr.byte(i as c_int));
                if !statss[i].is_null() {
                    raise!(wr.string(statss[i]));
                } else {
                    raise!(wr.string(ptr::null()));
                }
            }
        }
        // The batched `MSG_WriteString` ops hold pointers into `statss`'s
        // elements, which point into the engine string table; flush before
        // the frame's locals die anyway, so nothing dangles.
        raise!(wr.flush());
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:630 SVFTE_CalcEntityDeltas

/// `sv_send.c:630`. `static` in C. `Mem_Realloc` aborts rather than
/// raising, so no status.
unsafe fn svfte_calc_entity_deltas(client: *mut Client) {
    // SAFETY: `client` is a live `client_t`; both entity lists are sorted
    // by `num`, which is what the merge below relies on.
    unsafe {
        let vm = vm();
        if ((*client).numpendingentities as c_int) < (*vm).num_edicts {
            let newmax = ((*vm).num_edicts + 64) as usize;
            (*client).pendingentities_bits = c::Mem_Realloc(
                (*client).pendingentities_bits.cast::<c_void>(),
                core::mem::size_of::<c_uint>() * newmax,
            )
            .cast::<c_uint>();
            ptr::write_bytes(
                (*client)
                    .pendingentities_bits
                    .add((*client).numpendingentities),
                0,
                newmax - (*client).numpendingentities,
            );
            (*client).numpendingentities = newmax;
        }

        if (*(*client).pendingentities_bits & UF_REMOVE) != 0 {
            (*client).numpreviousentities = 0;
            *(*client).pendingentities_bits = UF_REMOVE;
        }

        let mut news = SNAPSHOT_ENTSTATE;
        let newstop = news.wrapping_add(SNAPSHOT_NUMENTS);
        let mut olds = (*client).previousentities;
        let oldstop = if !olds.is_null() {
            olds.add((*client).numpreviousentities)
        } else {
            ptr::null_mut()
        };

        loop {
            if olds == oldstop && news == newstop {
                break;
            }
            if news == newstop || (olds != oldstop && (*olds).num < (*news).num) {
                *(*client).pendingentities_bits.add((*olds).num as usize) = UF_REMOVE;
                olds = olds.add(1);
            } else if olds == oldstop || (news != newstop && (*news).num < (*olds).num) {
                *(*client).pendingentities_bits.add((*news).num as usize) = UF_RESET;
                news = news.add(1);
            } else {
                let slot = (*client).pendingentities_bits.add((*news).num as usize);
                if (*slot & UF_REMOVE) != 0 {
                    *slot = (*slot & !UF_REMOVE) | UF_RESET2;
                }
                *slot |= msgfte_delta_calc_bits(
                    ptr::addr_of!((*olds).state),
                    ptr::addr_of!((*news).state),
                );
                news = news.add(1);
                olds = olds.add(1);
            }
        }

        // Swap the two buffers rather than copying (as the C does).
        let olds = (*client).previousentities;
        let oldstop = if !olds.is_null() {
            olds.add((*client).maxpreviousentities)
        } else {
            ptr::null_mut()
        };

        (*client).previousentities = SNAPSHOT_ENTSTATE;
        (*client).numpreviousentities = SNAPSHOT_NUMENTS;
        (*client).maxpreviousentities = SNAPSHOT_MAXENTS;

        SNAPSHOT_ENTSTATE = olds;
        SNAPSHOT_NUMENTS = 0;
        SNAPSHOT_MAXENTS = if !olds.is_null() {
            oldstop.offset_from(olds) as usize
        } else {
            0
        };
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:698 SVFTE_WriteEntitiesToClient

/// `sv_send.c:698`. `static` in C. Statusized: writes and `EDICT_NUM` can
/// raise.
///
/// COMPAT: ADR-010. `frame->timestamp = qcvm->time` narrows the `double`
/// server clock to `float` on assignment.
unsafe fn svfte_write_entities_to_client(
    client: *mut Client,
    wr: &mut Writer,
    overflowsize: usize,
) -> Raise {
    // SAFETY: `client` is a live `client_t`; `wr` wraps the caller's
    // sizebuf, whose `maxsize` this function temporarily lowers exactly as
    // the C does.
    unsafe {
        let vm = vm();
        let sequence = g::NET_QSocketGetSequenceOut((*client).netconnection.cast::<c_void>());
        let mut origmaxsize: c_int = 0;
        raise!(wr.maxsize(&mut origmaxsize));
        let origmaxsize = origmaxsize as usize;
        let frame = (*client)
            .frames
            .add((sequence & ((*client).numframes as c_int - 1)) as usize);
        (*frame).sequence = sequence;
        (*frame).timestamp = (*vm).time as c_float;

        raise!(wr.set_maxsize(overflowsize as c_int));

        let mut state = (*client).previousentities;
        let stateend = state.wrapping_add((*client).numpreviousentities);

        raise!(wr.byte(SVCFTE_UPDATEENTITIES));

        (*frame).numents = 0;
        if ((*client).protocol_pext2 & PEXT2_PREDINFO) != 0 {
            raise!(wr.short((*client).lastmovemessage & 0xffff));
        }
        raise!(wr.float((*frame).timestamp));

        let mut entnum = (*client).snapshotresume as usize;
        while entnum < (*client).numpendingentities {
            let entbits = *(*client).pendingentities_bits.add(entnum);
            if (entbits & !UF_RESET2) == 0 {
                entnum += 1;
                continue;
            }

            let mut rollbacksize: c_int = 0;
            raise!(wr.cursize(&mut rollbacksize));
            *(*client).pendingentities_bits.add(entnum) = 0;
            let mut logbits: c_uint = 0;
            if (entbits & UF_REMOVE) != 0 {
                if entnum > 0x3fff {
                    raise!(wr.short((0xc000 | (entnum & 0x3fff)) as c_int));
                    raise!(wr.byte(((entnum >> 14) & 0xff) as c_int));
                } else {
                    raise!(wr.short((0x8000 | entnum) as c_int));
                }
                logbits = UF_REMOVE;
            } else {
                while state < stateend && ((*state).num as usize) < entnum {
                    state = state.add(1);
                }
                if state < stateend && (*state).num as usize == entnum {
                    let netbits: c_uint;
                    if (entbits & UF_RESET2) != 0 {
                        logbits = entbits & !(UF_RESET | UF_RESET2);
                        let mut ed: *mut c_void = ptr::null_mut();
                        raise!(w::World_Glue_EdictNum(entnum as c_int, &mut ed));
                        netbits = UF_RESET
                            | msgfte_delta_calc_bits(
                                ptr::addr_of!((*ed.cast::<Edict>()).baseline),
                                ptr::addr_of!((*state).state),
                            );
                    } else if (entbits & UF_RESET) != 0 {
                        *(*client).pendingentities_bits.add(entnum) = UF_RESET2;
                        let mut ed: *mut c_void = ptr::null_mut();
                        raise!(w::World_Glue_EdictNum(entnum as c_int, &mut ed));
                        netbits = UF_RESET
                            | msgfte_delta_calc_bits(
                                ptr::addr_of!((*ed.cast::<Edict>()).baseline),
                                ptr::addr_of!((*state).state),
                            );
                        logbits = UF_RESET;
                    } else {
                        logbits = entbits;
                        netbits = entbits;
                    }

                    if entnum >= 0x4000 {
                        raise!(wr.short((0x4000 | (entnum & 0x3fff)) as c_int));
                        raise!(wr.byte(((entnum >> 14) & 0xff) as c_int));
                    } else {
                        raise!(wr.short(entnum as c_int));
                    }
                    raise!(msgfte_write_entity_update(
                        wr,
                        netbits,
                        ptr::addr_of!((*state).state),
                        (*client).protocol_pext2,
                        sv.protocolflags,
                    ));
                }
            }

            let mut cursize: c_int = 0;
            raise!(wr.cursize(&mut cursize));
            if cursize as usize + 2 > origmaxsize {
                raise!(wr.set_cursize(rollbacksize));
                *(*client).pendingentities_bits.add(entnum) = entbits;
                break;
            }
            if (*frame).numents == (*frame).maxents {
                (*frame).maxents += 64;
                (*frame).ents = c::Mem_Realloc(
                    (*frame).ents.cast::<c_void>(),
                    core::mem::size_of::<DeltaFrameEnt>() * (*frame).maxents as usize,
                )
                .cast::<DeltaFrameEnt>();
            }
            let slot = (*frame).ents.add((*frame).numents as usize);
            (*slot).num = entnum as c_uint;
            (*slot).ebits = logbits;
            (*slot).csqcbits = 0;
            (*frame).numents += 1;

            entnum += 1;
        }
        raise!(wr.set_maxsize(origmaxsize as c_int));
        raise!(wr.short(0));

        (*client).snapshotresume = entnum as c_uint;

        let mut cursize: c_int = 0;
        raise!(wr.cursize(&mut cursize));
        if cursize > 1024 && dev_peakstats.packetsize <= 1024 {
            g::SvSend_Glue_WarnPacket(cursize);
        }
        dev_stats.packetsize = cursize;
        dev_peakstats.packetsize = if cursize > dev_peakstats.packetsize {
            cursize
        } else {
            dev_peakstats.packetsize
        };
    }
    0
}

/// `protocol.h:219` / `:405-412`
const ENTALPHA_ZERO: u8 = 1;
const EFLAGS_STEP: u8 = 1;
const EFLAGS_ONGROUND: u8 = 128;
/// `progs.h:43`
const MAX_ENT_LEAFS: c_uint = 32;

// ---------------------------------------------------------------------------
// sv_send.c:810 SV_BuildEntityState

/// `sv_send.c:810`. Statusized: `NUM_FOR_EDICT` raises.
///
/// COMPAT: ADR-010. `val->vector[i] * 32` is f32 (`float * int`), truncated
/// on the store into the `byte` colormod. The expression
/// `(ent->v.nextthink - qcvm->time) * 1000` computes in *double* because
/// `qcvm->time` is `double`, then `Q_rint` adds another `double` 0.5. Every
/// float-to-integer store here is undefined in C when out of range and
/// saturating in Rust (ADR-010 rule 8).
unsafe fn sv_build_entity_state(ent: *mut Edict, state: *mut EntityState) -> Raise {
    // SAFETY: `ent` is a live edict and `state` a live `entity_state_t`.
    unsafe {
        let vm = vm();
        (*state).eflags = 0;
        if sv_use_pred_think_pos(ent) {
            (*state).origin = (*ent).predthinkpos;
        } else {
            (*state).origin = (*ent).v.origin;
        }
        (*state).angles = (*ent).v.angles;
        (*state).modelindex = (*ent).v.modelindex as u16;
        (*state).frame = (*ent).v.frame as u16;
        (*state).colormap = (*ent).v.colormap as u8;
        (*state).skin = (*ent).v.skin as u8;

        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.scale);
        (*state).scale = if !val.is_null() {
            entscale_encode(ev_float(val)) as u8
        } else {
            ENTSCALE_DEFAULT as u8
        };

        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.alpha);
        (*state).alpha = if !val.is_null() {
            entalpha_encode(ev_float(val)) as u8
        } else {
            (*ent).alpha
        };

        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.colormod);
        if !val.is_null()
            && (ev_vector(val, 0) != 0.0 || ev_vector(val, 1) != 0.0 || ev_vector(val, 2) != 0.0)
        {
            (*state).colormod[0] = (ev_vector(val, 0) * 32.0) as u8;
            (*state).colormod[1] = (ev_vector(val, 1) * 32.0) as u8;
            (*state).colormod[2] = (ev_vector(val, 2) * 32.0) as u8;
        } else {
            (*state).colormod[0] = 32;
            (*state).colormod[1] = 32;
            (*state).colormod[2] = 32;
        }

        (*state).traileffectnum = if (*vm).extfields.traileffectnum >= 0 {
            ev_float(ph::GetEdictFieldValue(
                ent.cast::<c_void>(),
                (*vm).extfields.traileffectnum,
            )) as u16
        } else {
            0
        };
        (*state).emiteffectnum = if (*vm).extfields.emiteffectnum >= 0 {
            ev_float(ph::GetEdictFieldValue(
                ent.cast::<c_void>(),
                (*vm).extfields.emiteffectnum,
            )) as u16
        } else {
            0
        };

        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.tag_entity);
        if !val.is_null() && ev_int(val) != 0 {
            let mut num: c_int = 0;
            raise!(num_for_edict(prog_to_edict(vm, ev_int(val)), &mut num));
            (*state).tagentity = num as u16;
        } else {
            (*state).tagentity = 0;
        }

        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.tag_index);
        (*state).tagindex = if !val.is_null() {
            ev_float(val) as u8
        } else {
            0
        };

        (*state).effects = ((*ent).v.effects as c_int & sv.effectsmask) as c_uint;
        let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.modelflags);
        if !val.is_null() {
            (*state).effects |= (ev_float(val) as u32) << 24;
        }
        if (*ent).v.movetype == MOVETYPE_STEP {
            (*state).eflags |= EFLAGS_STEP;
        }

        (*state).pmovetype = 0;
        (*state).velocity = [0; 3];

        // `LERP_BANDAID` (`protocol.h:33`).
        (*state).lerp = if (*ent).sendinterval || (*ent).sendinterval_default {
            (q_rint_d(((*ent).v.nextthink as f64 - (*vm).time) * 1000.0) + 1) as u16
        } else {
            0
        };
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:865 SVFTE_BuildSnapshotForClient

/// `sv_send.c:865`. `static` in C. Statusized: `PR_GetString`,
/// `SV_BuildEntityState` and `SV_FatPVS`'s edict walk can raise.
///
/// COMPAT: ADR-010. `ent->v.velocity[i] * 8` is `float * int`, i.e. f32,
/// then truncated into the `short` velocity slot (undefined out of range in
/// C, saturating in Rust -- rule 8).
unsafe fn svfte_build_snapshot_for_client(client: *mut Client) -> Raise {
    // SAFETY: `client` is a live `client_t` with a live `edict`; the edict
    // walk uses `qcvm->edict_size` strides exactly as `NEXT_EDICT` does.
    unsafe {
        let vm = vm();
        let mut maxentities = (*client).limit_entities;
        let clent = (*client).edict;
        let proged = edict_to_prog(vm, clent);

        let mut ents = SNAPSHOT_ENTSTATE;
        let mut numents: usize = 0;
        let mut maxents = SNAPSHOT_MAXENTS;

        let org: [c_float; 3] = [
            (*clent).v.origin[0] + (*clent).v.view_ofs[0],
            (*clent).v.origin[1] + (*clent).v.view_ofs[1],
            (*clent).v.origin[2] + (*clent).v.view_ofs[2],
        ];
        let pvs = sv_fat_pvs(org.as_ptr(), (*vm).worldmodel.cast::<QModel>());

        if maxentities > (*vm).num_edicts as c_uint {
            maxentities = (*vm).num_edicts as c_uint;
        }

        let mut ent = next_edict(vm, (*vm).edicts);
        let mut e: c_uint = 1;
        while e < maxentities {
            let mut eflags: u8 = 0;
            let mut invisible = false;
            if ent != clent {
                // The C's `!ent->v.modelindex || !PR_GetString (...)[0]`
                // short-circuits, so a zero modelindex must not reach the
                // (raise-capable) string lookup at all.
                let mut nomodel = (*ent).v.modelindex == 0.0;
                if !nomodel {
                    let mut model: *const c_char = ptr::null();
                    raise!(get_string((*ent).v.model, &mut model));
                    nomodel = *model == 0;
                }
                if nomodel {
                    invisible = true;
                } else {
                    let parent = ent;
                    if (*parent).num_leafs != 0 {
                        let mut i: c_uint = 0;
                        while i < (*parent).num_leafs {
                            let leaf = (*parent).leafnums[i as usize];
                            if (*pvs.wrapping_offset((leaf >> 3) as isize) & (1 << (leaf & 7))) != 0
                            {
                                break;
                            }
                            i += 1;
                        }
                        if i == (*parent).num_leafs && (*parent).num_leafs < MAX_ENT_LEAFS {
                            invisible = true;
                        }
                    }
                }
            }

            if !invisible {
                let val =
                    ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.nodrawtoclient);
                if !val.is_null() && ev_int(val) == proged {
                    invisible = true;
                }
            }
            if !invisible {
                let val =
                    ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.drawonlytoclient);
                if !val.is_null() && ev_int(val) != 0 && ev_int(val) != proged {
                    invisible = true;
                }
            }

            if invisible {
                e += 1;
                ent = next_edict(vm, ent);
                continue;
            }

            if numents == maxents {
                maxents += 64;
                ents = c::Mem_Realloc(
                    ents.cast::<c_void>(),
                    maxents * core::mem::size_of::<EntityNumState>(),
                )
                .cast::<EntityNumState>();
            }

            let slot = ents.add(numents);
            (*slot).num = e;
            raise!(sv_build_entity_state(ent, ptr::addr_of_mut!((*slot).state)));
            if (*slot).state.modelindex as c_uint >= (*client).limit_models {
                (*slot).state.modelindex = 0;
            }
            if ent == clent {
                (*slot).state.pmovetype = 0;
                if ((*ent).v.flags as c_int & FL_ONGROUND) != 0 {
                    eflags |= EFLAGS_ONGROUND;
                }
                (*slot).state.velocity[0] = ((*ent).v.velocity[0] * 8.0) as i16;
                (*slot).state.velocity[1] = ((*ent).v.velocity[1] * 8.0) as i16;
                (*slot).state.velocity[2] = ((*ent).v.velocity[2] * 8.0) as i16;
            } else if (*slot).state.alpha == ENTALPHA_ZERO && (*ent).v.effects == 0.0 {
                e += 1;
                ent = next_edict(vm, ent);
                continue;
            }
            (*slot).state.eflags |= eflags;

            numents += 1;

            e += 1;
            ent = next_edict(vm, ent);
        }

        SNAPSHOT_ENTSTATE = ents;
        SNAPSHOT_NUMENTS = numents;
        SNAPSHOT_MAXENTS = maxents;
    }
    0
}

/// `protocol.h:36` / `:211-214`
const PROTOCOL_FITZQUAKE: c_uint = 666;
const B_LARGEMODEL: c_int = 1 << 0;
const B_LARGEFRAME: c_int = 1 << 1;
const B_ALPHA: c_int = 1 << 2;
const B_SCALE: c_int = 1 << 3;

// ---------------------------------------------------------------------------
// sv_send.c:963 MSG_WriteStaticOrBaseLine

/// `sv_send.c:963`. Statusized: every write can raise.
unsafe fn msg_write_static_or_baseline(
    wr: &mut Writer,
    idx: c_int,
    state: *const EntityState,
    protocol_pext2: c_uint,
    protocol: c_uint,
    protocolflags: c_uint,
) -> Raise {
    // SAFETY: `state` is a live `entity_state_t`; `wr` wraps a live sizebuf.
    unsafe {
        if (protocol_pext2 & PEXT2_REPLACEMENTDELTAS) != 0 {
            if idx >= 0 {
                raise!(wr.byte(SVCFTE_SPAWNBASELINE2));
                raise!(wr.short(idx));
            } else {
                raise!(wr.byte(SVCFTE_SPAWNSTATIC2));
            }
            let bits = msgfte_delta_calc_bits(ptr::addr_of!(nullentitystate), state);
            raise!(msgfte_write_entity_update(
                wr,
                bits,
                state,
                protocol_pext2,
                protocolflags
            ));
        } else {
            let mut bits: c_int = 0;
            if protocol == PROTOCOL_FITZQUAKE || protocol == PROTOCOL_RMQ {
                if (*state).modelindex > 255 {
                    bits |= B_LARGEMODEL;
                }
                if (*state).frame > 255 {
                    bits |= B_LARGEFRAME;
                }
                if (*state).alpha as c_int != ENTALPHA_DEFAULT {
                    bits |= B_ALPHA;
                }
                if (*state).scale as c_int != ENTSCALE_DEFAULT && protocol == PROTOCOL_RMQ {
                    bits |= B_SCALE;
                }
            }
            if idx >= 0 {
                raise!(wr.byte(if bits != 0 {
                    SVC_SPAWNBASELINE2
                } else {
                    SVC_SPAWNBASELINE
                }));
                raise!(wr.entity(idx, protocol_pext2));
            } else {
                raise!(wr.byte(if bits != 0 {
                    SVC_SPAWNSTATIC2
                } else {
                    SVC_SPAWNSTATIC
                }));
            }

            if bits != 0 {
                raise!(wr.byte(bits));
            }

            if (bits & B_LARGEMODEL) != 0 {
                raise!(wr.short((*state).modelindex as c_int));
            } else {
                raise!(wr.byte((*state).modelindex as c_int));
            }

            if (bits & B_LARGEFRAME) != 0 {
                raise!(wr.short((*state).frame as c_int));
            } else {
                raise!(wr.byte((*state).frame as c_int));
            }

            raise!(wr.byte((*state).colormap as c_int));
            raise!(wr.byte((*state).skin as c_int));
            for i in 0..3 {
                raise!(wr.coord((*state).origin[i], protocolflags));
                raise!(wr.angle((*state).angles[i], protocolflags));
            }
            if (bits & B_ALPHA) != 0 {
                raise!(wr.byte((*state).alpha as c_int));
            }
            if (bits & B_SCALE) != 0 {
                raise!(wr.byte((*state).scale as c_int));
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1044 SV_AddToFatPVS

/// `sv_send.c:1044`. Cannot raise (`Mod_LeafPVS` is a pure lookup).
///
/// COMPAT: ADR-010. `DotProduct (org, plane->normal) - plane->dist` is
/// evaluated left to right in f32 -- `((a0*b0 + a1*b1) + a2*b2) - dist` --
/// and must not be reassociated or widened.
unsafe fn sv_add_to_fat_pvs(org: *const c_float, mut node: *mut MNode, worldmodel: *mut QModel) {
    // SAFETY: `node` walks the world BSP; `fatpvs` is at least `fatbytes`
    // long, and `Mod_LeafPVS` returns a buffer of the same length.
    unsafe {
        loop {
            if (*node).contents < 0 {
                if (*node).contents != CONTENTS_SOLID {
                    FATPVS_ANY = true;
                    let pvs = g::Mod_LeafPVS(node.cast::<c_void>(), worldmodel.cast::<c_void>());
                    let mut i: c_int = 0;
                    while i < FATBYTES - 3 {
                        let dst = FATPVS.offset(i as isize).cast::<u32>();
                        let src = pvs.offset(i as isize).cast::<u32>();
                        dst.write_unaligned(dst.read_unaligned() | src.read_unaligned());
                        i += 4;
                    }
                }
                return;
            }

            let plane = (*node).plane;
            let n = (*plane).normal;
            let d: c_float =
                (*org.add(0) * n[0] + *org.add(1) * n[1] + *org.add(2) * n[2]) - (*plane).dist;
            if d > 8.0 {
                node = (*node).children[0];
            } else if d < -8.0 {
                node = (*node).children[1];
            } else {
                sv_add_to_fat_pvs(org, (*node).children[0], worldmodel);
                node = (*node).children[1];
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:1085 SV_FatPVS

/// `sv_send.c:1085`. Cannot raise: the only failure path is `Sys_Error`,
/// which aborts (ADR-009).
unsafe fn sv_fat_pvs(org: *const c_float, worldmodel: *mut QModel) -> *mut u8 {
    // SAFETY: `worldmodel` is the live world `qmodel_t`.
    unsafe {
        FATBYTES = ((*worldmodel).numleafs + 31) / 8;
        if FATPVS.is_null() || FATBYTES > FATPVS_CAPACITY {
            FATPVS_CAPACITY = FATBYTES;
            FATPVS = c::Mem_Realloc(FATPVS.cast::<c_void>(), FATPVS_CAPACITY as usize).cast::<u8>();
            if FATPVS.is_null() {
                g::SvSend_Glue_FatPvsAllocFailed(FATPVS_CAPACITY);
            }
        }

        ptr::write_bytes(FATPVS, 0, FATBYTES as usize);
        FATPVS_ANY = false;
        sv_add_to_fat_pvs(org, (*worldmodel).nodes, worldmodel);
        if !FATPVS_ANY {
            ptr::write_bytes(FATPVS, 0xff, FATBYTES as usize);
        }
        FATPVS
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:1115 SV_VisibleToClient

/// `sv_send.c:1115`. Cannot raise.
unsafe fn sv_visible_to_client(
    client: *mut Edict,
    test: *mut Edict,
    worldmodel: *mut QModel,
) -> bool {
    // SAFETY: both edicts are live; `leafnums` is valid for `num_leafs`.
    unsafe {
        let org: [c_float; 3] = [
            (*client).v.origin[0] + (*client).v.view_ofs[0],
            (*client).v.origin[1] + (*client).v.view_ofs[1],
            (*client).v.origin[2] + (*client).v.view_ofs[2],
        ];
        let pvs = sv_fat_pvs(org.as_ptr(), worldmodel);

        for i in 0..(*test).num_leafs as usize {
            let leaf = (*test).leafnums[i];
            if (*pvs.offset((leaf >> 3) as isize) & (1 << (leaf & 7))) != 0 {
                return true;
            }
        }
        false
    }
}

/// `server.h:232-237` -- the movetypes the netsort heuristic tests.
const MOVETYPE_FLY: c_float = 5.0;
const MOVETYPE_TOSS: c_float = 6.0;
const MOVETYPE_FLYMISSILE: c_float = 9.0;
const MOVETYPE_BOUNCE: c_float = 10.0;

// ---------------------------------------------------------------------------
// sv_send.c:1143 SV_WriteEntitiesToClient

/// `sv_send.c:1143`. Statusized: `PR_GetString`, `NUM_FOR_EDICT` and every
/// write can raise.
///
/// COMPAT: ADR-010, three float sites.
/// * `miss = origin[i] - ent->baseline.origin[i]` is f32; the comparisons
///   against the `double` literals `-0.1`/`0.1` promote it.
/// * `dist = 8.f * sqrt (sqrt (dist / size))` calls the *double* `sqrt`
///   twice: the f32 quotient promotes, `8.f` promotes, and the f64 product
///   narrows back on the assignment to the `float dist`. The `sqrt` used is
///   the platform libm, never `f64::sqrt`.
/// * `Q_rint ((ent->v.nextthink - qcvm->time) * 255)` computes in double
///   (`qcvm->time` is `double`); the enclosing `CLAMP (0, ..., 255)` selects
///   the `int` arm because every operand is `int`.
unsafe fn sv_write_entities_to_client(
    client: *mut Client,
    wr: &mut Writer,
    overflowsize: usize,
) -> Raise {
    // SAFETY: `client` is a live `client_t` with a live `edict`; the edict
    // walk uses `qcvm->edict_size` strides exactly as `NEXT_EDICT` does, and
    // the four netsort scratch arrays are `MAX_EDICTS` long, which
    // `client->limit_entities` bounds.
    unsafe {
        let vm = vm();
        let clent = (*client).edict;
        let mut maxedict = (*vm).num_edicts as c_uint;
        let mut origmaxsize: c_int = 0;
        raise!(wr.maxsize(&mut origmaxsize));
        let origmaxsize = origmaxsize as usize;
        let mut sort = cvar_value(ptr::addr_of!(m::sv_netsort)) > 1.0;

        if cvar_value(ptr::addr_of!(m::sv_netsort)) == 1.0
            && dev_overflows.packetsize + 10.0 > m::realtime
        {
            sort = true;
        }

        raise!(wr.set_maxsize(overflowsize as c_int));

        if maxedict > (*client).limit_entities {
            maxedict = (*client).limit_entities;
        }

        let org: [c_float; 3] = [
            (*clent).v.origin[0] + (*clent).v.view_ofs[0],
            (*clent).v.origin[1] + (*clent).v.view_ofs[1],
            (*clent).v.origin[2] + (*clent).v.view_ofs[2],
        ];
        let pvs = sv_fat_pvs(org.as_ptr(), (*vm).worldmodel.cast::<QModel>());

        let mut forward: [c_float; 3] = [0.0; 3];
        let mut right: [c_float; 3] = [0.0; 3];
        let mut up: [c_float; 3] = [0.0; 3];
        g::SvSend_Glue_AngleVectors(
            (*clent).v.v_angle.as_ptr(),
            forward.as_mut_ptr(),
            right.as_mut_ptr(),
            up.as_mut_ptr(),
        );

        NET_EDICT_BINS = [0; 256];

        let mut clentnum: c_int = 0;
        raise!(num_for_edict(clent, &mut clentnum));
        if sort {
            NET_EDICTS[0] = clentnum as u16;
            NET_EDICT_DISTS[0] = 0;
            NET_EDICT_BINS[0] = 1;
        } else {
            NET_EDICTS_SORTED[0] = clentnum as u16;
        }
        let mut numents: c_uint = 1;

        let mut ent = next_edict(vm, (*vm).edicts);
        let mut e: c_uint = 1;
        while e < maxedict {
            if ent != clent {
                let mut model: *const c_char = ptr::null();
                // `!ent->v.modelindex || !PR_GetString (ent->v.model)[0]`
                // short-circuits, so a zero modelindex must not reach the
                // (raise-capable) string lookup at all.
                let mut nomodel = (*ent).v.modelindex == 0.0;
                if !nomodel {
                    raise!(get_string((*ent).v.model, &mut model));
                    nomodel = *model == 0;
                }
                if nomodel || (*ent).v.modelindex as c_uint >= (*client).limit_models {
                    e += 1;
                    ent = next_edict(vm, ent);
                    continue;
                }

                let mut i: c_uint = 0;
                while i < (*ent).num_leafs {
                    let leaf = (*ent).leafnums[i as usize];
                    if (*pvs.offset((leaf >> 3) as isize) & (1 << (leaf & 7))) != 0 {
                        break;
                    }
                    i += 1;
                }
                if i == (*ent).num_leafs && (*ent).num_leafs < MAX_ENT_LEAFS {
                    e += 1;
                    ent = next_edict(vm, ent);
                    continue;
                }

                if sort {
                    let mut dist: c_float = 0.0;
                    let mut size: c_float = 0.0;
                    #[allow(clippy::needless_range_loop)] // three parallel arrays
                    for i in 0..3 {
                        let mut delta =
                            clamp_f((*ent).v.absmin[i], org[i], (*ent).v.absmax[i]) - org[i];
                        dist += delta * delta;
                        delta = (*ent).v.absmax[i] - (*ent).v.absmin[i];
                        size += delta * delta;
                    }
                    size = max_f(1.0, size);

                    if size < 50.0 && (*ent).v.touch != 0 {
                        if (*ent).v.movetype == MOVETYPE_FLYMISSILE
                            || (*ent).v.movetype == MOVETYPE_FLY
                        {
                            let to_self: [c_float; 3] = [
                                org[0] - (*ent).v.origin[0],
                                org[1] - (*ent).v.origin[1],
                                org[2] - (*ent).v.origin[2],
                            ];
                            let direction: c_float = (*ent).v.velocity[0] * to_self[0]
                                + (*ent).v.velocity[1] * to_self[1]
                                + (*ent).v.velocity[2] * to_self[2];
                            size = if direction > 0.0
                                || !g::strstr(model, c"miss".as_ptr()).is_null()
                                || !g::strstr(model, c"rocket".as_ptr()).is_null()
                            {
                                3072.0
                            } else {
                                768.0
                            };
                        } else if (*ent).v.movetype == MOVETYPE_BOUNCE
                            || (*ent).v.movetype == MOVETYPE_TOSS
                        {
                            size = if (*ent).v.nextthink > 0.0
                                && g::strstr(model, c"gib".as_ptr()).is_null()
                            {
                                3072.0
                            } else {
                                768.0
                            };
                        }
                    }

                    dist = (8.0f64 * c::libm::sqrt(c::libm::sqrt((dist / size) as f64))) as c_float;
                    NET_EDICT_DISTS[numents as usize] = min_f(dist, 255.0) as c_int as u8;
                    NET_EDICTS[numents as usize] = e as u16;

                    dist = 0.0;
                    for i in 0..3 {
                        dist += (if forward[i] < 0.0 {
                            (*ent).v.absmin[i]
                        } else {
                            (*ent).v.absmax[i]
                        } - org[i])
                            * forward[i];
                    }
                    if dist < 0.0 {
                        NET_EDICT_DISTS[numents as usize] |= 128;
                    }

                    NET_EDICT_BINS[NET_EDICT_DISTS[numents as usize] as usize] += 1;
                } else {
                    NET_EDICTS_SORTED[numents as usize] = e as u16;
                }

                numents += 1;
            }
            e += 1;
            ent = next_edict(vm, ent);
        }

        if sort {
            let mut acc: c_uint = 0;
            #[allow(clippy::needless_range_loop)] // in-place prefix sum
            for i in 0..256usize {
                let tmp = NET_EDICT_BINS[i];
                NET_EDICT_BINS[i] = acc as c_int;
                acc = acc.wrapping_add(tmp as c_uint);
            }

            for e in 0..numents as usize {
                let bin = NET_EDICT_DISTS[e] as usize;
                NET_EDICTS_SORTED[NET_EDICT_BINS[bin] as usize] = NET_EDICTS[e];
                NET_EDICT_BINS[bin] += 1;
            }
        }

        #[allow(clippy::needless_range_loop)] // `NET_EDICTS_SORTED` is a static
        for j in 0..numents as usize {
            let e = NET_EDICTS_SORTED[j] as c_uint;
            let ent = edict_num(vm, e as c_int);

            let mut rollbacksize: c_int = 0;
            raise!(wr.cursize(&mut rollbacksize));

            let mut bits: c_int = 0;

            let origin: [c_float; 3] = if sv_use_pred_think_pos(ent) {
                (*ent).predthinkpos
            } else {
                (*ent).v.origin
            };

            #[allow(clippy::needless_range_loop)] // two parallel arrays
            for i in 0..3 {
                let miss: c_float = origin[i] - (*ent).baseline.origin[i];
                if (miss as f64) < -0.1 || (miss as f64) > 0.1 {
                    bits |= U_ORIGIN1 << i;
                }
            }

            if (*ent).v.angles[0] != (*ent).baseline.angles[0] {
                bits |= U_ANGLE1;
            }
            if (*ent).v.angles[1] != (*ent).baseline.angles[1] {
                bits |= U_ANGLE2;
            }
            if (*ent).v.angles[2] != (*ent).baseline.angles[2] {
                bits |= U_ANGLE3;
            }

            if (*ent).v.movetype == MOVETYPE_STEP {
                bits |= U_STEP;
            }

            if (*ent).baseline.colormap as c_float != (*ent).v.colormap {
                bits |= U_COLORMAP;
            }
            if (*ent).baseline.skin as c_float != (*ent).v.skin {
                bits |= U_SKIN;
            }
            if (*ent).baseline.frame as c_float != (*ent).v.frame {
                bits |= U_FRAME;
            }
            if ((*ent).baseline.effects ^ (*ent).v.effects as c_int as c_uint)
                & sv.effectsmask as c_uint
                != 0
            {
                bits |= U_EFFECTS;
            }
            if (*ent).baseline.modelindex as c_float != (*ent).v.modelindex {
                bits |= U_MODEL;
            }

            let proged = edict_to_prog(vm, clent);

            let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.nodrawtoclient);
            if !val.is_null() && ev_int(val) == proged {
                continue;
            }

            let val =
                ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.drawonlytoclient);
            if !val.is_null() && ev_int(val) != 0 && ev_int(val) != proged {
                continue;
            }

            let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.alpha);
            if !val.is_null() {
                (*ent).alpha = entalpha_encode(ev_float(val)) as u8;
            }

            if (*ent).alpha == ENTALPHA_ZERO && ((*ent).v.effects as c_int & sv.effectsmask) == 0 {
                continue;
            }

            let val = ph::GetEdictFieldValue(ent.cast::<c_void>(), (*vm).extfields.scale);
            let scale: c_float = if !val.is_null() {
                entscale_encode(ev_float(val)) as c_float
            } else {
                ENTSCALE_DEFAULT as c_float
            };

            if sv.protocol != PROTOCOL_NETQUAKE {
                if (*ent).baseline.alpha != (*ent).alpha {
                    bits |= U_ALPHA;
                }
                if sv.protocol == PROTOCOL_RMQ {
                    if (*ent).baseline.scale as c_float != scale {
                        bits |= U_SCALE;
                    }
                } else if ENTSCALE_DEFAULT as c_float != scale {
                    bits |= U_SCALE;
                }
                if bits & U_FRAME != 0 && (*ent).v.frame as c_int > 255 {
                    bits |= U_FRAME2;
                }
                if bits & U_MODEL != 0 && (*ent).v.modelindex as c_int > 255 {
                    bits |= U_MODEL2;
                }
                if (*ent).sendinterval
                    || ((*ent).sendinterval_default
                        && (*client).limit_unreliable > DATAGRAM_MTU as c_uint)
                {
                    bits |= U_LERPFINISH;
                }
                if bits >= 65536 {
                    bits |= U_EXTEND1;
                }
                if bits >= 16777216 {
                    bits |= U_EXTEND2;
                }
            }

            if e >= 256 {
                bits |= U_LONGENTITY;
            }
            if bits >= 256 {
                bits |= U_MOREBITS;
            }

            raise!(wr.byte((bits | U_SIGNAL) & 0xff));

            if bits & U_MOREBITS != 0 {
                raise!(wr.byte((bits >> 8) & 0xff));
            }
            if bits & U_EXTEND1 != 0 {
                raise!(wr.byte((bits >> 16) & 0xff));
            }
            if bits & U_EXTEND2 != 0 {
                raise!(wr.byte((bits >> 24) & 0xff));
            }

            if bits & U_LONGENTITY != 0 {
                raise!(wr.short(e as c_int));
            } else {
                raise!(wr.byte(e as c_int));
            }

            if bits & U_MODEL != 0 {
                raise!(wr.byte((*ent).v.modelindex as c_int & 0xff));
            }
            if bits & U_FRAME != 0 {
                raise!(wr.byte((*ent).v.frame as c_int & 0xff));
            }
            if bits & U_COLORMAP != 0 {
                raise!(wr.byte((*ent).v.colormap as c_int));
            }
            if bits & U_SKIN != 0 {
                raise!(wr.byte((*ent).v.skin as c_int));
            }
            if bits & U_EFFECTS != 0 {
                raise!(wr.byte((*ent).v.effects as c_int & sv.effectsmask));
            }
            if bits & U_ORIGIN1 != 0 {
                raise!(wr.coord(origin[0], sv.protocolflags));
            }
            if bits & U_ANGLE1 != 0 {
                raise!(wr.angle((*ent).v.angles[0], sv.protocolflags));
            }
            if bits & U_ORIGIN2 != 0 {
                raise!(wr.coord(origin[1], sv.protocolflags));
            }
            if bits & U_ANGLE2 != 0 {
                raise!(wr.angle((*ent).v.angles[1], sv.protocolflags));
            }
            if bits & U_ORIGIN3 != 0 {
                raise!(wr.coord(origin[2], sv.protocolflags));
            }
            if bits & U_ANGLE3 != 0 {
                raise!(wr.angle((*ent).v.angles[2], sv.protocolflags));
            }

            if bits & U_ALPHA != 0 {
                raise!(wr.byte((*ent).alpha as c_int));
            }
            if bits & U_SCALE != 0 {
                raise!(wr.byte(scale as c_int));
            }
            if bits & U_FRAME2 != 0 {
                raise!(wr.byte((*ent).v.frame as c_int >> 8));
            }
            if bits & U_MODEL2 != 0 {
                raise!(wr.byte((*ent).v.modelindex as c_int >> 8));
            }
            if bits & U_LERPFINISH != 0 {
                raise!(wr.byte(clamp_i(
                    0,
                    q_rint_d(((*ent).v.nextthink as f64 - (*vm).time) * 255.0),
                    255,
                ) as u8 as c_int));
            }

            let mut cursize: c_int = 0;
            raise!(wr.cursize(&mut cursize));
            if cursize as usize > origmaxsize {
                raise!(wr.set_cursize(rollbacksize));
                if dev_overflows.packetsize == 0.0
                    || dev_overflows.packetsize + CONSOLE_RESPAM_TIME < m::realtime
                {
                    g::SvSend_Glue_WarnOverflow();
                    dev_overflows.packetsize = m::realtime;
                }
                break;
            }
        }

        raise!(wr.set_maxsize(origmaxsize as c_int));

        let mut cursize: c_int = 0;
        raise!(wr.cursize(&mut cursize));
        let mut maxsize: c_int = 0;
        raise!(wr.maxsize(&mut maxsize));
        if cursize > 1024 && dev_peakstats.packetsize <= 1024 {
            g::SvSend_Glue_WarnPacketMax(cursize, maxsize);
        }
        dev_stats.packetsize = cursize;
        dev_peakstats.packetsize = if cursize > dev_peakstats.packetsize {
            cursize
        } else {
            dev_peakstats.packetsize
        };
    }
    0
}

/// `server.h:291`
const EF_MUZZLEFLASH: c_int = 2;
/// `protocol.h:235`
const DEFAULT_VIEWHEIGHT: c_float = 22.0;

// ---------------------------------------------------------------------------
// sv_send.c:1476 SV_CleanupEnts

/// `sv_send.c:1476`.
unsafe fn sv_cleanup_ents() {
    // SAFETY: the walk uses `qcvm->edict_size` strides, exactly `NEXT_EDICT`.
    unsafe {
        let vm = vm();
        let mut ent = next_edict(vm, (*vm).edicts);
        let mut e: c_int = 1;
        while e < (*vm).num_edicts {
            (*ent).v.effects = ((*ent).v.effects as c_int & !EF_MUZZLEFLASH) as c_float;
            e += 1;
            ent = next_edict(vm, ent);
        }
    }
}

// ---------------------------------------------------------------------------
// sv_send.c:1495 SV_WriteDamageToMessage

/// `sv_send.c:1495`. Statusized: the writes and `SV_SetIdealPitch` raise.
///
/// COMPAT: ADR-010. `other->v.origin[i] + 0.5 * (other->v.mins[i] +
/// other->v.maxs[i])` mixes a `double` literal into a `float` expression:
/// the `mins + maxs` sum is f32, the `0.5 *` product and the outer add are
/// `double`, and the result narrows only when it is passed as
/// `MSG_WriteCoord`'s `float` argument.
unsafe fn sv_write_damage_to_message(ent: *mut Edict, wr: &mut Writer) -> Raise {
    // SAFETY: `ent` is a live edict; `dmg_inflictor` is a prog offset the C
    // dereferences without a range check either.
    unsafe {
        let vm = vm();
        if (*ent).v.dmg_take != 0.0 || (*ent).v.dmg_save != 0.0 {
            let other = prog_to_edict(vm, (*ent).v.dmg_inflictor);
            raise!(wr.byte(SVC_DAMAGE));
            if (*ent).v.dmg_save > 255.0 {
                (*ent).v.dmg_save = 255.0;
            }
            raise!(wr.byte((*ent).v.dmg_save as c_int));

            if (*ent).v.dmg_take > 255.0 {
                (*ent).v.dmg_take = 255.0;
            }
            raise!(wr.byte((*ent).v.dmg_take as c_int));

            for i in 0..3 {
                let c = (*other).v.origin[i] as f64
                    + 0.5 * (((*other).v.mins[i] + (*other).v.maxs[i]) as f64);
                raise!(wr.coord(c as c_float, sv.protocolflags));
            }

            (*ent).v.dmg_take = 0.0;
            (*ent).v.dmg_save = 0.0;
        }

        // The damage bytes are already in the sizebuf when the C reaches
        // `SV_SetIdealPitch`, so they must be flushed before a call that can
        // longjmp past this frame.
        raise!(wr.flush());
        raise!(g::SvSend_Glue_SetIdealPitch());

        if (*ent).v.fixangle != 0.0 {
            raise!(wr.byte(SVC_SETANGLE));
            for i in 0..3 {
                raise!(wr.angle((*ent).v.angles[i], sv.protocolflags));
            }
            (*ent).v.fixangle = 0.0;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1544 SV_WriteClientdataToMessage

/// `sv_send.c:1544`. Statusized: `PR_GetString` and every write raise.
///
/// COMPAT: ADR-010. `MSG_WriteChar (msg, ent->v.velocity[i] / 16)` divides
/// in f32 (`float / int`) and converts to `int` at the call, which is
/// undefined in C when out of range and saturating in Rust (rule 8).
unsafe fn sv_write_clientdata_to_message(client: *mut Client, wr: &mut Writer) -> Raise {
    // SAFETY: `client` is a live `client_t` with a live `edict`.
    unsafe {
        let ent = (*client).edict;
        let mut weaponmodel: *const c_char = ptr::null();
        raise!(get_string((*ent).v.weaponmodel, &mut weaponmodel));
        let mut weaponmodelindex = g::SvSend_Glue_ModelIndex(weaponmodel) as c_uint;

        if weaponmodelindex >= (*client).limit_models {
            weaponmodelindex = 0;
        }

        let mut bits: c_int = 0;

        if (*ent).v.view_ofs[2] != DEFAULT_VIEWHEIGHT {
            bits |= SU_VIEWHEIGHT;
        }
        if (*ent).v.idealpitch != 0.0 {
            bits |= SU_IDEALPITCH;
        }

        let val = ph::GetEdictFieldValue(
            ent.cast::<c_void>(),
            ph::ED_FindFieldOffset(c"items2".as_ptr()),
        );
        let items: c_int = if !val.is_null() {
            (*ent).v.items as c_int | (ev_float(val) as c_int) << 23
        } else {
            (*ent).v.items as c_int | ((*globals()).serverflags as c_int) << 28
        };

        bits |= SU_ITEMS;

        if (*ent).v.flags as c_int & FL_ONGROUND != 0 {
            bits |= SU_ONGROUND;
        }
        if (*ent).v.waterlevel >= 2.0 {
            bits |= SU_INWATER;
        }

        for i in 0..3 {
            if (*ent).v.punchangle[i] != 0.0 {
                bits |= SU_PUNCH1 << i;
            }
            if (*ent).v.velocity[i] != 0.0 {
                bits |= SU_VELOCITY1 << i;
            }
        }

        if (*ent).v.weaponframe != 0.0 {
            bits |= SU_WEAPONFRAME;
        }
        if (*ent).v.armorvalue != 0.0 {
            bits |= SU_ARMOR;
        }
        bits |= SU_WEAPON;

        if sv.protocol != PROTOCOL_NETQUAKE {
            if bits & SU_WEAPON != 0 && weaponmodelindex > 255 {
                bits |= SU_WEAPON2;
            }
            if (*ent).v.armorvalue as c_int > 255 {
                bits |= SU_ARMOR2;
            }
            if (*ent).v.currentammo as c_int > 255 {
                bits |= SU_AMMO2;
            }
            if (*ent).v.ammo_shells as c_int > 255 {
                bits |= SU_SHELLS2;
            }
            if (*ent).v.ammo_nails as c_int > 255 {
                bits |= SU_NAILS2;
            }
            if (*ent).v.ammo_rockets as c_int > 255 {
                bits |= SU_ROCKETS2;
            }
            if (*ent).v.ammo_cells as c_int > 255 {
                bits |= SU_CELLS2;
            }
            if bits & SU_WEAPONFRAME != 0 && (*ent).v.weaponframe as c_int > 255 {
                bits |= SU_WEAPONFRAME2;
            }
            if bits & SU_WEAPON != 0 && (*ent).alpha as c_int != ENTALPHA_DEFAULT {
                bits |= SU_WEAPONALPHA;
            }
            if bits >= 65536 {
                bits |= SU_EXTEND1;
            }
            if bits >= 16777216 {
                bits |= SU_EXTEND2;
            }
        }

        raise!(wr.byte(SVC_CLIENTDATA));
        raise!(wr.short(bits));

        if bits & SU_EXTEND1 != 0 {
            raise!(wr.byte(bits >> 16));
        }
        if bits & SU_EXTEND2 != 0 {
            raise!(wr.byte(bits >> 24));
        }

        if bits & SU_VIEWHEIGHT != 0 {
            raise!(wr.char_((*ent).v.view_ofs[2] as c_int));
        }
        if bits & SU_IDEALPITCH != 0 {
            raise!(wr.char_((*ent).v.idealpitch as c_int));
        }

        for i in 0..3 {
            if bits & (SU_PUNCH1 << i) != 0 {
                raise!(wr.char_((*ent).v.punchangle[i] as c_int));
            }
            if bits & (SU_VELOCITY1 << i) != 0 {
                raise!(wr.char_(((*ent).v.velocity[i] / 16.0) as c_int));
            }
        }

        raise!(wr.long(items));

        if bits & SU_WEAPONFRAME != 0 {
            raise!(wr.byte((*ent).v.weaponframe as c_int & 0xff));
        }
        if bits & SU_ARMOR != 0 {
            raise!(wr.byte((*ent).v.armorvalue as c_int & 0xff));
        }
        if bits & SU_WEAPON != 0 {
            raise!(wr.byte(weaponmodelindex as c_int & 0xff));
        }

        raise!(wr.short((*ent).v.health as c_int));
        raise!(wr.byte((*ent).v.currentammo as c_int & 0xff));
        raise!(wr.byte((*ent).v.ammo_shells as c_int & 0xff));
        raise!(wr.byte((*ent).v.ammo_nails as c_int & 0xff));
        raise!(wr.byte((*ent).v.ammo_rockets as c_int & 0xff));
        raise!(wr.byte((*ent).v.ammo_cells as c_int & 0xff));

        if g::SvSend_Glue_StandardQuake() != 0 {
            raise!(wr.byte((*ent).v.weapon as c_int & 0xff));
        } else {
            let mut weapon: c_int = 0;
            for i in 0..32 {
                if ((*ent).v.weapon as c_int) & (1 << i) != 0 {
                    weapon = i;
                    break;
                }
            }
            raise!(wr.byte(weapon));
        }

        if bits & SU_WEAPON2 != 0 {
            raise!(wr.byte((weaponmodelindex >> 8) as c_int));
        }
        if bits & SU_ARMOR2 != 0 {
            raise!(wr.byte((*ent).v.armorvalue as c_int >> 8));
        }
        if bits & SU_AMMO2 != 0 {
            raise!(wr.byte((*ent).v.currentammo as c_int >> 8));
        }
        if bits & SU_SHELLS2 != 0 {
            raise!(wr.byte((*ent).v.ammo_shells as c_int >> 8));
        }
        if bits & SU_NAILS2 != 0 {
            raise!(wr.byte((*ent).v.ammo_nails as c_int >> 8));
        }
        if bits & SU_ROCKETS2 != 0 {
            raise!(wr.byte((*ent).v.ammo_rockets as c_int >> 8));
        }
        if bits & SU_CELLS2 != 0 {
            raise!(wr.byte((*ent).v.ammo_cells as c_int >> 8));
        }
        if bits & SU_WEAPONFRAME2 != 0 {
            raise!(wr.byte((*ent).v.weaponframe as c_int >> 8));
        }
        if bits & SU_WEAPONALPHA != 0 {
            raise!(wr.byte((*ent).alpha as c_int));
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1709 SV_PresendClientDatagram

/// `sv_send.c:1709`. Statusized: the snapshot builder's `PR_GetString` and
/// `NUM_FOR_EDICT` raise.
unsafe fn sv_presend_client_datagram(client: *mut Client) -> Raise {
    // SAFETY: `client` is a live `client_t`.
    unsafe {
        if (*client).netconnection.is_null() {
            return 0;
        }
        if !(*client).spawned {
            return 0;
        }
        if (*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
            return 0;
        }
        raise!(svfte_build_snapshot_for_client(client));
        svfte_calc_entity_deltas(client);
        (*client).snapshotresume = 0;
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1729 SV_ParticleSize

/// `sv_send.c:1729`. `static` in C.
unsafe fn sv_particle_size(buf: *const u8) -> c_int {
    // SAFETY: `buf` points inside `sv.datagram_buf`, at a byte the caller
    // has already bounds-checked against `sv.datagram.cursize`.
    unsafe {
        if *buf as c_int == SVC_PARTICLE {
            let mut coord_size = 2;
            if sv.protocolflags & PRFL_24BITCOORD != 0 {
                coord_size = 3;
            } else if sv.protocolflags & (PRFL_FLOATCOORD | PRFL_INT32COORD) != 0 {
                coord_size = 4;
            }
            6 + 3 * coord_size
        } else {
            0
        }
    }
}

/// `q_min` on two `unsigned int`s -- `q_min (MAX_DATAGRAM,
/// client->limit_unreliable)` selects the `unsigned int` arm because
/// `int + unsigned int` is `unsigned int` (`sv_send.c:1765`).
#[inline]
fn min_u(a: c_uint, b: c_uint) -> c_uint {
    if a < b {
        a
    } else {
        b
    }
}

/// `sv_send.c:1751`'s `static byte buf[MAX_DATAGRAM + 1000]`.
static mut SEND_DATAGRAM_BUF: [u8; MAX_DATAGRAM + 1000] = [0; MAX_DATAGRAM + 1000];

// ---------------------------------------------------------------------------
// sv_send.c:1749 SV_SendClientDatagram

/// `sv_send.c:1749`. Statusized: the writers, `PR_GetString`,
/// `SV_SetIdealPitch` and `SV_DropClient` all raise. `*out` receives the C
/// return value and is only meaningful when the status is `HOST_GUARD_OK`.
///
/// COMPAT: ADR-010. `MSG_WriteFloat (&msg, qcvm->time)` narrows the `double`
/// VM clock to `float` at the call, exactly as the C prototype does.
///
/// COMPAT: the C leaves `msg.overflowed` uninitialised (`sv_send.c:1752`).
/// `SZ_GetSpace` only ever writes that field, never reads it, so
/// zero-initialising here is observationally identical and avoids Rust UB.
unsafe fn sv_send_client_datagram(client: *mut Client, out: &mut bool) -> Raise {
    // SAFETY: `client` is a live `client_t`; `SEND_DATAGRAM_BUF` is the
    // single-threaded server's scratch datagram, matching the C `static`.
    unsafe {
        let vm = vm();
        *out = true;

        if (*client).netconnection.is_null() {
            g::SvSend_Glue_SzClear(ptr::addr_of_mut!((*client).datagram).cast::<c_void>());
            return 0;
        }

        let mut msg = SizeBuf {
            allowoverflow: false,
            overflowed: false,
            data: ptr::addr_of_mut!(SEND_DATAGRAM_BUF).cast::<u8>(),
            maxsize: min_u(MAX_DATAGRAM as c_uint, (*client).limit_unreliable) as c_int,
            cursize: 0,
        };
        let bufsize = MAX_DATAGRAM + 1000;
        let msgp = ptr::addr_of_mut!(msg);
        let mut wr = Writer::new(msgp);

        host_client = client;
        if (*client).spawned {
            g::SvSend_Glue_SetPlayer((*client).edict.cast::<c_void>());

            if (*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS != 0 {
                raise!(sv_write_damage_to_message((*client).edict, &mut wr));
                if (*client).protocol_pext2 & PEXT2_PREDINFO == 0 {
                    raise!(sv_write_clientdata_to_message(client, &mut wr));
                } else {
                    raise!(svfte_write_stats(client, &mut wr));
                }
                raise!(svfte_write_entities_to_client(client, &mut wr, bufsize));

                // `SVFTE_WriteEntitiesToClient` advances `snapshotresume`
                // through the raw pointer, which clippy cannot see.
                #[allow(clippy::while_immutable_condition)]
                while (*client).snapshotresume < (*client).numpendingentities as c_uint {
                    raise!(wr.flush());
                    g::NET_SendUnreliableMessage(
                        (*client).netconnection.cast::<c_void>(),
                        msgp.cast::<c_void>(),
                    );
                    g::SvSend_Glue_SzClear(msgp.cast::<c_void>());
                    raise!(svfte_write_entities_to_client(client, &mut wr, bufsize));
                }
            } else {
                raise!(wr.byte(SVC_TIME));
                raise!(wr.float((*vm).time as c_float));
                if (*client).protocol_pext2 & PEXT2_PREDINFO != 0 {
                    raise!(wr.short((*client).lastmovemessage & 0xffff));
                }
                raise!(sv_write_entities_to_client(client, &mut wr, bufsize));
            }

            // copy the private datagram if there is space
            let mut cursize: c_int = 0;
            raise!(wr.cursize(&mut cursize));
            if (*client).datagram.cursize != 0 && !(*client).datagram.overflowed {
                if cursize + (*client).datagram.cursize < msg.maxsize {
                    raise!(wr.sz_write(
                        (*client).datagram.data.cast::<c_void>(),
                        (*client).datagram.cursize,
                    ));
                } else if (*client).datagram.cursize < msg.maxsize {
                    raise!(wr.flush());
                    g::NET_SendUnreliableMessage(
                        (*client).netconnection.cast::<c_void>(),
                        msgp.cast::<c_void>(),
                    );
                    g::SvSend_Glue_SzClear(msgp.cast::<c_void>());
                    raise!(wr.sz_write(
                        (*client).datagram.data.cast::<c_void>(),
                        (*client).datagram.cursize,
                    ));
                }
            }
            g::SvSend_Glue_SzClear(ptr::addr_of_mut!((*client).datagram).cast::<c_void>());

            // copy the server datagram if there is space
            raise!(wr.cursize(&mut cursize));
            if cursize + sv.datagram.cursize < msg.maxsize {
                raise!(wr.sz_write(sv.datagram.data.cast::<c_void>(), sv.datagram.cursize));
            } else if sv.datagram.cursize != 0 {
                let mut position: c_int = 0;
                loop {
                    if sv.datagram.cursize <= position {
                        break;
                    }
                    let size = sv_particle_size(sv.datagram.data.offset(position as isize));
                    if size == 0 {
                        break;
                    }
                    raise!(wr.cursize(&mut cursize));
                    if cursize + size < msg.maxsize {
                        raise!(wr.sz_write(
                            sv.datagram.data.offset(position as isize).cast::<c_void>(),
                            size,
                        ));
                        position += size;
                    } else {
                        raise!(wr.flush());
                        g::NET_SendUnreliableMessage(
                            (*client).netconnection.cast::<c_void>(),
                            msgp.cast::<c_void>(),
                        );
                        g::SvSend_Glue_SzClear(msgp.cast::<c_void>());
                    }
                }
                let remaining = sv.datagram.cursize - position;
                raise!(wr.cursize(&mut cursize));
                if cursize + remaining < msg.maxsize {
                    raise!(wr.sz_write(
                        sv.datagram.data.offset(position as isize).cast::<c_void>(),
                        remaining,
                    ));
                } else if remaining < msg.maxsize {
                    raise!(wr.flush());
                    g::NET_SendUnreliableMessage(
                        (*client).netconnection.cast::<c_void>(),
                        msgp.cast::<c_void>(),
                    );
                    g::SvSend_Glue_SzClear(msgp.cast::<c_void>());
                    raise!(wr.sz_write(
                        sv.datagram.data.offset(position as isize).cast::<c_void>(),
                        remaining,
                    ));
                }
            }

            if (*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS == 0 {
                // cannibalize client->datagram (cleared above) to get an
                // exact size
                let mut dwr = Writer::new(ptr::addr_of_mut!((*client).datagram));
                raise!(sv_write_damage_to_message((*client).edict, &mut dwr));
                raise!(sv_write_clientdata_to_message(client, &mut dwr));
                raise!(dwr.flush());

                raise!(wr.cursize(&mut cursize));
                if cursize + (*client).datagram.cursize > msg.maxsize {
                    raise!(wr.flush());
                    g::NET_SendUnreliableMessage(
                        (*client).netconnection.cast::<c_void>(),
                        msgp.cast::<c_void>(),
                    );
                    g::SvSend_Glue_SzClear(msgp.cast::<c_void>());
                }
                raise!(wr.sz_write(
                    (*client).datagram.data.cast::<c_void>(),
                    (*client).datagram.cursize,
                ));
                raise!(wr.flush());
                g::SvSend_Glue_SzClear(ptr::addr_of_mut!((*client).datagram).cast::<c_void>());
            }
        }

        // send the datagram
        let mut cursize: c_int = 0;
        raise!(wr.cursize(&mut cursize));
        if cursize != 0
            && g::NET_SendUnreliableMessage(
                (*client).netconnection.cast::<c_void>(),
                msgp.cast::<c_void>(),
            ) == -1
        {
            raise!(g::SvSend_Glue_DropClient(false));
            *out = false;
            return 0;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1881 SV_UpdateToReliableMessages

/// `sv_send.c:1881`. Statusized: the writes raise.
unsafe fn sv_update_to_reliable_messages() -> Raise {
    // SAFETY: `svs.clients` is a `svs.maxclients`-long array of live
    // `client_t`, each with a live `edict`.
    unsafe {
        let mut i: c_int = 0;
        host_client = svs.clients;
        while i < svs.maxclients {
            if (*host_client).old_frags != (*(*host_client).edict).v.frags as c_int {
                let mut j: c_int = 0;
                let mut client = svs.clients;
                while j < svs.maxclients {
                    if (*client).knowntoqc {
                        let mut wr = Writer::new(ptr::addr_of_mut!((*client).message));
                        raise!(wr.byte(SVC_UPDATEFRAGS));
                        raise!(wr.byte(i));
                        raise!(wr.short((*(*host_client).edict).v.frags as c_int));
                        raise!(wr.flush());
                    }
                    j += 1;
                    client = client.add(1);
                }

                (*host_client).old_frags = (*(*host_client).edict).v.frags as c_int;
            }
            i += 1;
            host_client = host_client.add(1);
        }

        let mut j: c_int = 0;
        let mut client = svs.clients;
        while j < svs.maxclients {
            if (*client).active {
                let mut wr = Writer::new(ptr::addr_of_mut!((*client).message));
                raise!(wr.sz_write(
                    sv.reliable_datagram.data.cast::<c_void>(),
                    sv.reliable_datagram.cursize,
                ));
                raise!(wr.flush());
            }
            j += 1;
            client = client.add(1);
        }

        g::SvSend_Glue_SzClear(ptr::addr_of_mut!(sv.reliable_datagram).cast::<c_void>());
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1922 SV_SendNop

/// `sv_send.c:1922`. Statusized: the write and `SV_DropClient` raise.
///
/// COMPAT: the C leaves `msg.allowoverflow` and `msg.overflowed`
/// uninitialised (`sv_send.c:1924`). Only one byte is written into a
/// four-byte buffer, so `SZ_GetSpace` never reaches the `allowoverflow`
/// test; zero-initialising is observationally identical and avoids Rust UB.
unsafe fn sv_send_nop(client: *mut Client) -> Raise {
    // SAFETY: `client` is a live `client_t` with a live `netconnection`.
    unsafe {
        let mut buf: [u8; 4] = [0; 4];
        let mut msg = SizeBuf {
            allowoverflow: false,
            overflowed: false,
            data: buf.as_mut_ptr(),
            maxsize: buf.len() as c_int,
            cursize: 0,
        };
        let msgp = ptr::addr_of_mut!(msg);
        let mut wr = Writer::new(msgp);
        raise!(wr.char_(SVC_NOP));
        raise!(wr.flush());

        if g::NET_SendUnreliableMessage(
            (*client).netconnection.cast::<c_void>(),
            msgp.cast::<c_void>(),
        ) == -1
        {
            raise!(g::SvSend_Glue_DropClient(false));
        }
        (*client).last_message = m::realtime;
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:1937 the prespawn senders

/// `sv_send.c:1937`.
fn sv_send_prespawn_model_precaches() -> bool {
    false
}

/// `sv_send.c:1941`. Statusized: the writes raise.
unsafe fn sv_send_prespawn_sound_precaches(out: &mut bool) -> Raise {
    // SAFETY: `host_client` is a live `client_t` during a send pass;
    // `sv.sound_precache` is a `MAX_SOUNDS` array of NUL-terminated strings.
    unsafe {
        let mut idx: c_uint = (*host_client).signon_sounds;
        let mut wr = Writer::new(ptr::addr_of_mut!((*host_client).message));
        let mut maxsize_i: c_int = 0;
        raise!(wr.maxsize(&mut maxsize_i));
        let maxsize = maxsize_i as usize; // we can go quite large
        if (*host_client).protocol_pext2 == 0 {
            *out = false; // unsupported by this client...
            return 0;
        }
        while idx < (*host_client).limit_sounds {
            let name = sv.sound_precache[idx as usize];
            if !name.is_null() {
                let mut cursize: c_int = 0;
                raise!(wr.cursize(&mut cursize));
                if cursize as usize + 4 + g::strlen(name) > maxsize {
                    break;
                }
                raise!(wr.byte(SVCDP_PRECACHE));
                raise!(wr.short((0x8000 | idx) as c_int));
                raise!(wr.string(name));
            }
            idx += 1;
        }
        raise!(wr.flush());
        (*host_client).signon_sounds = idx;
        *out = idx < (*host_client).limit_sounds;
    }
    0
}

/// `sv_send.c:1960`. Statusized: the writes raise.
unsafe fn sv_send_prespawn_particle_precaches(mut idx: c_int, out: &mut c_int) -> Raise {
    // SAFETY: `host_client` is a live `client_t`; `sv.particle_precache` is a
    // `MAX_PARTICLETYPES` array of NUL-terminated strings.
    unsafe {
        let mut wr = Writer::new(ptr::addr_of_mut!((*host_client).message));
        let mut maxsize_i: c_int = 0;
        raise!(wr.maxsize(&mut maxsize_i));
        let maxsize = maxsize_i as usize; // we can go quite large
        if (*host_client).protocol_pext2 == 0 {
            *out = -1; // unsupported by this client.
            return 0;
        }
        loop {
            if idx == MAX_PARTICLETYPES as c_int {
                *out = -1;
                raise!(wr.flush());
                return 0;
            }
            let name = sv.particle_precache[idx as usize];
            if !name.is_null() {
                let mut cursize: c_int = 0;
                raise!(wr.cursize(&mut cursize));
                if (cursize + 4) as usize + g::strlen(name) > maxsize {
                    break;
                }
                raise!(wr.byte(SVCDP_PRECACHE));
                raise!(wr.short((0x4000 | idx as c_uint) as c_int));
                raise!(wr.string(name));
            }
            idx += 1;
        }
        raise!(wr.flush());
        *out = idx;
    }
    0
}

/// `memcmp (&nullentitystate, state, sizeof (nullentitystate)) != 0`.
///
/// The C compares the whole `entity_state_t` object representation, padding
/// included; this reproduces that byte-for-byte.
#[inline]
unsafe fn differs_from_nullentitystate(state: *const EntityState) -> bool {
    // SAFETY: `state` points at a live, fully initialised `entity_state_t`
    // (`sv.static_entities` and `edict_t::baseline` are both zeroed at
    // allocation), so reading its object representation is defined.
    unsafe {
        let n = core::mem::size_of::<EntityState>();
        let a = core::slice::from_raw_parts(ptr::addr_of!(nullentitystate).cast::<u8>(), n);
        let b = core::slice::from_raw_parts(state.cast::<u8>(), n);
        a != b
    }
}

/// `sv_send.c:1979`. Statusized: the writes raise.
unsafe fn sv_send_prespawn_statics(mut idx: c_int, out: &mut c_int) -> Raise {
    // SAFETY: `host_client` is live; `sv.static_entities` holds
    // `sv.num_statics` live `entity_state_t`.
    unsafe {
        let mut wr = Writer::new(ptr::addr_of_mut!((*host_client).message));
        let mut maxsize: c_int = 0;
        raise!(wr.maxsize(&mut maxsize));
        maxsize -= 128; // we can go quite large

        loop {
            if idx >= sv.num_statics {
                raise!(wr.flush());
                *out = -1;
                return 0;
            }
            let svent = sv.static_entities.offset(idx as isize);

            let mut cursize: c_int = 0;
            raise!(wr.cursize(&mut cursize));
            if cursize > maxsize {
                break;
            }
            idx += 1;

            if (*svent).modelindex as c_uint >= (*host_client).limit_models {
                continue;
            }
            if differs_from_nullentitystate(svent) {
                raise!(msg_write_static_or_baseline(
                    &mut wr,
                    -1,
                    svent,
                    (*host_client).protocol_pext2,
                    sv.protocol,
                    sv.protocolflags,
                ));
            }
        }
        raise!(wr.flush());
        *out = idx;
    }
    0
}

/// `sv_send.c:2001`. Statusized: the writes raise.
unsafe fn sv_send_ambient_sounds(mut idx: c_int, out: &mut c_int) -> Raise {
    // SAFETY: `host_client` is live; `sv.ambientsounds` holds
    // `sv.num_ambients` live `struct ambientsound_s`.
    unsafe {
        let mut wr = Writer::new(ptr::addr_of_mut!((*host_client).message));
        let mut maxsize: c_int = 0;
        raise!(wr.maxsize(&mut maxsize));
        maxsize -= 128; // we can go quite large

        loop {
            if idx >= sv.num_ambients {
                raise!(wr.flush());
                *out = -1;
                return 0;
            }
            let snd = sv.ambientsounds.offset(idx as isize);

            let mut cursize: c_int = 0;
            raise!(wr.cursize(&mut cursize));
            if cursize > maxsize {
                break;
            }
            idx += 1;

            if (*snd).soundindex >= (*host_client).limit_sounds {
                continue;
            }

            let large = (*snd).soundindex > 255;
            if large {
                raise!(wr.byte(SVC_SPAWNSTATICSOUND2)); // johnfitz -- PROTOCOL_FITZQUAKE
            } else {
                raise!(wr.byte(SVC_SPAWNSTATICSOUND));
            }
            for i in 0..3usize {
                raise!(wr.coord((*snd).origin[i], sv.protocolflags));
            }
            if large {
                raise!(wr.short((*snd).soundindex as c_int));
            } else {
                raise!(wr.byte((*snd).soundindex as c_int));
            }
            raise!(wr.byte(clamp_f(0.0, (*snd).volume * 255.0, 255.0) as c_int));
            raise!(wr.byte(clamp_f(0.0, (*snd).attenuation * 64.0, 255.0) as c_int));
        }
        raise!(wr.flush());
        *out = idx;
    }
    0
}

/// `sv_send.c:2038`. Statusized: the writes raise.
unsafe fn sv_send_prespawn_baselines(mut idx: c_int, out: &mut c_int) -> Raise {
    // SAFETY: `host_client` is live; `qcvm` addresses the server VM whose
    // edict arena holds `qcvm->num_edicts` live edicts.
    unsafe {
        let vm = vm();
        let mut wr = Writer::new(ptr::addr_of_mut!((*host_client).message));
        let mut maxsize: c_int = 0;
        raise!(wr.maxsize(&mut maxsize));
        maxsize -= 128; // we can go quite large

        loop {
            if idx >= (*vm).num_edicts {
                raise!(wr.flush());
                *out = -1;
                return 0;
            }
            let svent = edict_num(vm, idx);

            let mut cursize: c_int = 0;
            raise!(wr.cursize(&mut cursize));
            if cursize > maxsize {
                break;
            }

            if differs_from_nullentitystate(ptr::addr_of!((*svent).baseline)) {
                raise!(msg_write_static_or_baseline(
                    &mut wr,
                    idx,
                    ptr::addr_of!((*svent).baseline),
                    (*host_client).protocol_pext2,
                    sv.protocol,
                    sv.protocolflags,
                ));
            }

            idx += 1;
        }
        raise!(wr.flush());
        *out = idx;
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:2065 SV_SendClientMessages

/// `sv_send.c:2065`. Statusized: every write and `SV_DropClient` raise.
unsafe fn sv_send_client_messages() -> Raise {
    // SAFETY: `svs.clients` is a `svs.maxclients`-long array of live
    // `client_t`; the loop mirrors the C's `host_client` cursor exactly.
    unsafe {
        // update frags, names, etc
        raise!(sv_update_to_reliable_messages());

        let mut i: c_int = 0;
        host_client = svs.clients;
        while i < svs.maxclients {
            if (*host_client).active {
                // generates client snapshots (and updates csqc pending flags)
                raise!(sv_presend_client_datagram(host_client));
            }
            i += 1;
            host_client = host_client.add(1);
        }

        // build individual updates
        let mut i: c_int = 0;
        host_client = svs.clients;
        while i < svs.maxclients {
            if (*host_client).active {
                raise!(sv_send_client_messages_one(host_client));
            }
            i += 1;
            host_client = host_client.add(1);
        }

        // clear muzzle flashes
        sv_cleanup_ents();
    }
    0
}

/// The body of `SV_SendClientMessages`'s second loop (`sv_send.c:2081-2190`),
/// lifted into its own function so the C's `continue` is a plain `return`.
/// The global `host_client` equals `client` for the whole call, exactly as in
/// the C, so the callees that read it observe the same cursor.
unsafe fn sv_send_client_messages_one(client: *mut Client) -> Raise {
    // SAFETY: `client` is a live, active `client_t` and is the current value
    // of the global `host_client`.
    unsafe {
        let mut sent = false;
        raise!(sv_send_client_datagram(client, &mut sent));
        if !sent {
            return 0;
        }

        if !(*client).spawned {
            // the player isn't totally in the game yet
            // send small keepalive messages if too much time has passed
            // send a full message when the next signon stage has been
            // requested
            // some other message data (name changes, etc) may accumulate
            // between signon stages
            if (*client).sendsignon == 0 {
                if m::realtime - (*client).last_message > 5.0 {
                    raise!(sv_send_nop(client));
                }
                return 0; // don't send out non-signon messages
            }
            if (*client).sendsignon == PRESPAWN_MODELS && !sv_send_prespawn_model_precaches() {
                (*client).signonidx = 0;
                (*client).sendsignon += 1;
            }
            if (*client).sendsignon == PRESPAWN_SOUNDS {
                let mut more = false;
                raise!(sv_send_prespawn_sound_precaches(&mut more));
                if !more {
                    (*client).signonidx = 0;
                    (*client).sendsignon += 1;
                }
            }
            if (*client).sendsignon == PRESPAWN_PARTICLES {
                let mut idx: c_int = 0;
                raise!(sv_send_prespawn_particle_precaches(
                    (*client).signonidx,
                    &mut idx
                ));
                (*client).signonidx = idx;
                if (*client).signonidx < 0 {
                    (*client).signonidx = 0;
                    (*client).sendsignon += 1;
                }
            }
            if (*client).sendsignon == PRESPAWN_BASELINES {
                let mut idx: c_int = 0;
                raise!(sv_send_prespawn_baselines((*client).signonidx, &mut idx));
                (*client).signonidx = idx;
                if (*client).signonidx < 0 {
                    (*client).signonidx = 0;
                    (*client).sendsignon += 1;
                }
            }
            if (*client).sendsignon == PRESPAWN_STATICS {
                let mut idx: c_int = 0;
                raise!(sv_send_prespawn_statics((*client).signonidx, &mut idx));
                (*client).signonidx = idx;
                if (*client).signonidx < 0 {
                    (*client).signonidx = 0;
                    (*client).sendsignon += 1;
                }
            }
            if (*client).sendsignon == PRESPAWN_AMBIENTS {
                let mut idx: c_int = 0;
                raise!(sv_send_ambient_sounds((*client).signonidx, &mut idx));
                (*client).signonidx = idx;
                if (*client).signonidx < 0 {
                    (*client).signonidx = 0;
                    (*client).sendsignon += 1;
                }
            }
            if (*client).sendsignon == PRESPAWN_SIGNONMSG {
                let mut wr = Writer::new(ptr::addr_of_mut!((*client).message));
                let mut cursize: c_int = 0;
                let mut maxsize: c_int = 0;
                raise!(wr.cursize(&mut cursize));
                raise!(wr.maxsize(&mut maxsize));
                if cursize + sv.signon.cursize + 2 < maxsize {
                    raise!(wr.sz_write(sv.signon.data.cast::<c_void>(), sv.signon.cursize));
                    raise!(wr.byte(SVC_SIGNONNUM));
                    raise!(wr.byte(2));
                    raise!(wr.flush());
                    (*client).sendsignon = PRESPAWN_FLUSH;
                }
            }
        }

        // check for an overflowed message.  Should only happen
        // on a very fucked up connection that backs up a lot, then
        // changes level
        let mut tail = Writer::new(ptr::addr_of_mut!((*client).message));
        let mut overflowed = false;
        raise!(tail.overflowed(&mut overflowed));
        if overflowed {
            g::SvSend_Glue_SzClear(ptr::addr_of_mut!((*client).message).cast::<c_void>());
            raise!(g::SvSend_Glue_DropClient(false));
            return 0;
        }

        let mut cursize: c_int = 0;
        raise!(tail.cursize(&mut cursize));
        if cursize != 0 || (*client).dropasap {
            if !m::NET_CanSendMessage((*client).netconnection.cast::<c_void>()) {
                // I_Printf: can't write
                return 0;
            }

            if (*client).dropasap {
                raise!(g::SvSend_Glue_DropClient(false)); // went to another level
            } else {
                if m::NET_SendMessage(
                    (*client).netconnection.cast::<c_void>(),
                    ptr::addr_of_mut!((*client).message).cast::<c::sizebuf_t>(),
                ) == -1
                {
                    // if the message couldn't send, kick off
                    raise!(g::SvSend_Glue_DropClient(false));
                }
                g::SvSend_Glue_SzClear(ptr::addr_of_mut!((*client).message).cast::<c_void>());
                (*client).last_message = m::realtime;
                if (*client).sendsignon == PRESPAWN_FLUSH {
                    (*client).sendsignon = PRESPAWN_DONE;
                }
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:2206 SV_CreateBaseline

/// `sv_send.c:2206`. Statusized: `PR_GetString` raises.
unsafe fn sv_create_baseline() -> Raise {
    // SAFETY: the ambient VM is the server's; `entnum` stays below
    // `qcvm->num_edicts` so every edict addressed here is live.
    unsafe {
        let vm = vm();
        let mut entnum: c_int = 0;
        while entnum < (*vm).num_edicts {
            // get the current server version
            let svent = edict_num(vm, entnum);
            if (*svent).free {
                entnum += 1;
                continue;
            }
            if entnum > svs.maxclients && (*svent).v.modelindex == 0.0 {
                entnum += 1;
                continue;
            }

            //
            // create entity baseline
            //
            (*svent).baseline = nullentitystate;
            (*svent).baseline.origin = (*svent).v.origin;
            (*svent).baseline.angles = (*svent).v.angles;
            (*svent).baseline.frame = (*svent).v.frame as u16;
            (*svent).baseline.skin = (*svent).v.skin as u8;
            if entnum > 0 && entnum <= svs.maxclients {
                (*svent).baseline.colormap = entnum as u8;
                (*svent).baseline.modelindex =
                    g::SvSend_Glue_ModelIndex(c"progs/player.mdl".as_ptr()) as u16;
            } else {
                (*svent).baseline.colormap = 0;
                let mut model: *const c_char = ptr::null();
                raise!(get_string((*svent).v.model, &mut model));
                (*svent).baseline.modelindex = g::SvSend_Glue_ModelIndex(model) as u16;
                let val = ph::GetEdictFieldValue(svent.cast::<c_void>(), (*vm).extfields.alpha);
                if !val.is_null() {
                    (*svent).baseline.alpha = entalpha_encode(ev_float(val)) as u8;
                } else {
                    (*svent).baseline.alpha = (*svent).alpha; // johnfitz -- alpha support
                }
                let val = ph::GetEdictFieldValue(svent.cast::<c_void>(), (*vm).extfields.scale);
                if !val.is_null() {
                    (*svent).baseline.scale = entscale_encode(ev_float(val)) as u8;
                }
            }

            // Spike -- baselines are now transmitted on a per-client basis.
            // FIXME: should merge the above with other edict->entity_state
            // copies (updates, baselines, spawnstatics)
            // 1) this allows per-client extensions.
            // 2) this avoids pre-generating a single signon buffer, splitting
            //    it over multiple packets, thereby allowing more than 3k or so
            //    entities

            entnum += 1;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// sv_send.c:2262 SV_SendReconnect

/// `sv_send.c:2262`. Statusized: the writes and `Cmd_ExecuteString` raise.
///
/// COMPAT: the C leaves `msg.allowoverflow` and `msg.overflowed`
/// uninitialised (`sv_send.c:2264-2268`). Eleven bytes go into a 128-byte
/// buffer, so `SZ_GetSpace` never reads either field; zero-initialising is
/// observationally identical and avoids Rust UB.
unsafe fn sv_send_reconnect() -> Raise {
    // SAFETY: `data` is a live local backing a stack sizebuf that never
    // escapes this frame.
    unsafe {
        let mut data: [u8; 128] = [0; 128];
        let mut msg = SizeBuf {
            allowoverflow: false,
            overflowed: false,
            data: data.as_mut_ptr(),
            cursize: 0,
            maxsize: data.len() as c_int,
        };
        let msgp = ptr::addr_of_mut!(msg);

        let mut wr = Writer::new(msgp);
        raise!(wr.char_(SVC_STUFFTEXT));
        raise!(wr.string(c"reconnect\n".as_ptr()));
        raise!(wr.flush());
        g::NET_SendToAll(msgp.cast::<c_void>(), 5.0);

        if !c::isDedicated {
            raise!(g::SvSend_Glue_ExecuteReconnect());
        }
    }
    0
}

// ---------------------------------------------------------------------------
// FFI exports
//
// Every parameter is `*mut c_void` or a primitive so cbindgen emits plain
// `void *` in build-rs/quake_rs.h and no mirror type leaks into the header.
// Each returns a Host_Guard status (ADR-009 rule 3): the caller in
// Quake/sv_send_glue.c -- a pure C frame -- feeds it to Host_Reraise.
// Nothing here may panic across the boundary.

/// `SV_CalcStats (client, statsi, statsf, statss)`.
///
/// # Safety
/// `client` is a live `client_t *`; the three arrays are `MAX_CL_STATS` long.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_calc_stats(
    client: *mut c_void,
    statsi: *mut c_void,
    statsf: *mut c_void,
    statss: *mut c_void,
) -> c_int {
    // SAFETY: the caller passes the C objects unchanged; the casts only
    // restore the mirror types.
    unsafe {
        sv_calc_stats(
            client.cast::<Client>(),
            statsi.cast::<c_int>(),
            statsf.cast::<c_float>(),
            statss.cast::<*const c_char>(),
        )
    }
}

/// `SVFTE_DestroyFrames (client)`.
///
/// # Safety
/// `client` is a live `client_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_destroy_frames(client: *mut c_void) -> c_int {
    // SAFETY: `client` is the caller's live `client_t`.
    unsafe { svfte_destroy_frames(client.cast::<Client>()) };
    0
}

/// `SVFTE_SetupFrames (client)`.
///
/// # Safety
/// `client` is a live `client_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_setup_frames(client: *mut c_void) -> c_int {
    // SAFETY: `client` is the caller's live `client_t`.
    unsafe { svfte_setup_frames(client.cast::<Client>()) };
    0
}

/// `SVFTE_Ack (client, sequence)`.
///
/// # Safety
/// `client` is a live `client_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_ack(client: *mut c_void, sequence: c_int) -> c_int {
    // SAFETY: `client` is the caller's live `client_t`.
    unsafe { svfte_ack(client.cast::<Client>(), sequence) };
    0
}

/// `SV_BuildEntityState (ent, state)`.
///
/// # Safety
/// `ent` is a live `edict_t *` and `state` a live `entity_state_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_build_entity_state(
    ent: *mut c_void,
    state: *mut c_void,
) -> c_int {
    // SAFETY: the casts restore the mirror types of the caller's objects.
    unsafe { sv_build_entity_state(ent.cast::<Edict>(), state.cast::<EntityState>()) }
}

/// `MSG_WriteStaticOrBaseLine (buf, idx, state, pext2, protocol, flags)`.
///
/// # Safety
/// `buf` is a live `sizebuf_t *` and `state` a live `entity_state_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_write_static_or_baseline(
    buf: *mut c_void,
    idx: c_int,
    state: *mut c_void,
    protocol_pext2: c_uint,
    protocol: c_uint,
    protocolflags: c_uint,
) -> c_int {
    // SAFETY: the casts restore the mirror types of the caller's objects.
    unsafe {
        let mut wr = Writer::new(buf.cast::<SizeBuf>());
        let r = msg_write_static_or_baseline(
            &mut wr,
            idx,
            state.cast::<EntityState>(),
            protocol_pext2,
            protocol,
            protocolflags,
        );
        if r != 0 {
            return r;
        }
        wr.flush()
    }
}

/// `SV_AddToFatPVS (org, node, worldmodel)`.
///
/// # Safety
/// `org` is a live `vec3_t`, `node` a live `mnode_t *`, `worldmodel` the
/// world `qmodel_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_add_to_fat_pvs(
    org: *mut c_void,
    node: *mut c_void,
    worldmodel: *mut c_void,
) -> c_int {
    // SAFETY: the casts restore the mirror types of the caller's objects.
    unsafe {
        sv_add_to_fat_pvs(
            org.cast::<c_float>(),
            node.cast::<MNode>(),
            worldmodel.cast::<QModel>(),
        )
    };
    0
}

/// `SV_FatPVS (org, worldmodel)`; the result lands in `*out`.
///
/// # Safety
/// `org` is a live `vec3_t`, `worldmodel` the world `qmodel_t *`, and `out`
/// a live `byte **`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_fat_pvs(
    org: *mut c_void,
    worldmodel: *mut c_void,
    out: *mut *mut c_void,
) -> c_int {
    // SAFETY: `out` is a live out-parameter owned by the caller's frame.
    unsafe {
        let p = sv_fat_pvs(org.cast::<c_float>(), worldmodel.cast::<QModel>());
        *out = p.cast::<c_void>();
    }
    0
}

/// `SV_VisibleToClient (client, test, worldmodel)`; the result lands in
/// `*out` as 0/1.
///
/// # Safety
/// Both edicts are live and `out` is a live `int *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_visible_to_client(
    client: *mut c_void,
    test: *mut c_void,
    worldmodel: *mut c_void,
    out: *mut c_int,
) -> c_int {
    // SAFETY: `out` is a live out-parameter owned by the caller's frame.
    unsafe {
        let v = sv_visible_to_client(
            client.cast::<Edict>(),
            test.cast::<Edict>(),
            worldmodel.cast::<QModel>(),
        );
        *out = v as c_int;
    }
    0
}

/// `SV_WriteEntitiesToClient (client, msg, overflowsize)`.
///
/// # Safety
/// `client` is a live `client_t *` and `msg` a live `sizebuf_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_write_entities_to_client(
    client: *mut c_void,
    msg: *mut c_void,
    overflowsize: usize,
) -> c_int {
    // SAFETY: the casts restore the mirror types of the caller's objects.
    unsafe {
        let mut wr = Writer::new(msg.cast::<SizeBuf>());
        let r = sv_write_entities_to_client(client.cast::<Client>(), &mut wr, overflowsize);
        if r != 0 {
            return r;
        }
        wr.flush()
    }
}

/// `SV_WriteClientdataToMessage (client, msg)`.
///
/// # Safety
/// `client` is a live `client_t *` and `msg` a live `sizebuf_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_write_clientdata_to_message(
    client: *mut c_void,
    msg: *mut c_void,
) -> c_int {
    // SAFETY: the casts restore the mirror types of the caller's objects.
    unsafe {
        let mut wr = Writer::new(msg.cast::<SizeBuf>());
        let r = sv_write_clientdata_to_message(client.cast::<Client>(), &mut wr);
        if r != 0 {
            return r;
        }
        wr.flush()
    }
}

/// `SV_PresendClientDatagram (client)`.
///
/// # Safety
/// `client` is a live `client_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_presend_client_datagram(client: *mut c_void) -> c_int {
    // SAFETY: `client` is the caller's live `client_t`.
    unsafe { sv_presend_client_datagram(client.cast::<Client>()) }
}

/// `SV_SendClientDatagram (client)`; the result lands in `*out` as 0/1.
///
/// # Safety
/// `client` is a live `client_t *` and `out` a live `int *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_send_client_datagram(
    client: *mut c_void,
    out: *mut c_int,
) -> c_int {
    // SAFETY: `out` is a live out-parameter owned by the caller's frame.
    unsafe {
        let mut v = false;
        let r = sv_send_client_datagram(client.cast::<Client>(), &mut v);
        *out = v as c_int;
        r
    }
}

/// `SV_SendClientMessages ()`.
///
/// # Safety
/// Called only with the server VM ambient and `svs.clients` populated.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_send_client_messages() -> c_int {
    // SAFETY: the C caller guarantees the server-state precondition.
    unsafe { sv_send_client_messages() }
}

/// `SV_CreateBaseline ()`.
///
/// # Safety
/// Called only with the server VM ambient.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_create_baseline() -> c_int {
    // SAFETY: the C caller guarantees the server-state precondition.
    unsafe { sv_create_baseline() }
}

/// `SV_SendReconnect ()`.
///
/// # Safety
/// Called only from the host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_svsend_send_reconnect() -> c_int {
    // SAFETY: the C caller guarantees the host-frame precondition.
    unsafe { sv_send_reconnect() }
}
