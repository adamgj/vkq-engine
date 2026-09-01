//! `Quake/cl_main.c` -- the client main loop (Rust migration Phase 7 M7,
//! T7.4). Pattern A whole-file swap: `Quake/cl_main_glue.c` is the C frame,
//! this module is the core, and `quake-c-sys`'s `cl_main` module declares the
//! glue.
//!
//! ADR-007: this module closes the client dual-view row. `cl` and `cls` are
//! defined here as `#[no_mangle] static mut`, exactly as `sv_main.rs` defines
//! `sv`/`svs`; `client.h`'s `extern` declarations bind the remaining C
//! translation units to the same storage, so there is one object, not two
//! views. Every *other* object `cl_main.c` defined keeps C storage in
//! `cl_main_glue.c` -- they have no dual-view problem and many C readers, so
//! moving them would be churn (the same call `cl_tent_glue.c` made for
//! `cl_beams[]`).
//!
//! ADR-009: `cl_main.c` has exactly three raise sites of its own --
//! `Host_Error ("CL_Connect: connect failed")` at `:232`,
//! `Host_Error ("CL_ReadFromServer: lost server connection")` at `:983` and
//! `Host_Error ("CL_SendCmd: lost server connection")` at `:1108`. They are
//! reported as [`RAISE_CONNECT_FAILED`], [`RAISE_LOST_READ`] and
//! [`RAISE_LOST_SEND`], and `cl_main_glue.c`'s `ClMain_Raise` turns them back
//! into the original `Host_Error` calls from a pure C frame. Every callee that
//! can itself `Host_Error`/`Host_EndGame` is reached through a
//! `ClMain_Glue_*` `Host_Guard` trampoline whose status is propagated
//! unchanged.
//!
//! ADR-010: `cl_main.c`'s only libm surface is `anglemod` (`:698`) and the
//! `mathlib` vector helpers, all of which are already `quake-math`; no
//! `f32`/`f64` method is used. Float widths and operation order are
//! reproduced literally -- notably `CL_LerpPoint`, where `f` and `frac` are
//! `float` but every term feeding them is `double`, and the `dl->die`
//! assignments, where a `double` sum is narrowed on store.
//!
//! ADR-005: the format specifiers reachable from this file are `%i`, `%3i`,
//! `%2i`, `%s`, `%d` and `%5.1f`. There is no `%g` or `%e`, so the Rust float
//! formatter's panic path is unreachable.

#![allow(non_upper_case_globals)]

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_main as g;
use quake_math::mathlib as m;
use quake_types::host::{
    Client, ClientState, ClientStatic, EntityOpaque, ScoreBoard, MAX_PARTICLETYPES,
};
use quake_types::progs::EntityState;

use crate::view::{Entity, RefDef};

// ---------------------------------------------------------------------------
// ADR-007 storage.

/// `cl_main.c:56` -- `client_static_t cls;`.
///
/// SAFETY: every field of `ClientStatic` is an integer, a float, a `bool`, a
/// raw pointer, or an array of those, so all-zero is a valid value. C's own
/// definition is a plain tentative definition and is zero-initialised too.
#[no_mangle]
pub static mut cls: ClientStatic = unsafe { core::mem::zeroed() };

/// `cl_main.c:57` -- `client_state_t cl;`.
///
/// SAFETY: as [`cls`]; the embedded `QcVm` is likewise all-POD.
#[no_mangle]
pub static mut cl: ClientState = unsafe { core::mem::zeroed() };

// ---------------------------------------------------------------------------
// ADR-009 plumbing.

/// A `Host_Guard` status, or one of the `RAISE_*` codes below.
type Raise = c_int;

/// `cl_main.c:232`.
const RAISE_CONNECT_FAILED: Raise = -101;
/// `cl_main.c:983`.
const RAISE_LOST_READ: Raise = -102;
/// `cl_main.c:1108`.
const RAISE_LOST_SEND: Raise = -103;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// engine constants (client.h / quakedef.h / protocol.h / keys.h / gl_model.h)

/// `client.h:68`
const SIGNONS: c_int = 4;
/// `client.h:99`
const MAX_DEMOS: c_int = 8;
/// `client.h:104-107` -- `cactive_t`.
const CA_DEDICATED: c_int = 0;
const CA_DISCONNECTED: c_int = 1;
const CA_CONNECTED: c_int = 2;
/// `quakedef.h:80-81`
const MIN_EDICTS: c_int = 256;
const MAX_EDICTS: c_int = 32000;
/// `quakedef.h:68`
const MAX_PHYSICS_FREQ: f64 = 72.0;
/// `quakedef.h` / `client.h`
const MAX_CL_STATS: usize = 256;
const MAX_LIGHTSTYLES: usize = 64;
const MAX_DLIGHTS: usize = 64;
const MAX_BEAMS: usize = 32;
const MAX_TEMP_ENTITIES: usize = 256;
/// `keys.h:136-142` -- `keydest_t`.
const KEY_MESSAGE: c_int = 2;
/// `protocol.h:347-350`
const CLC_NOP: c_int = 1;
const CLC_DISCONNECT: c_int = 2;
const CLC_STRINGCMD: c_int = 4;
/// `protocol.h:254`, `:264`, `:268`
const SVC_STUFFTEXT: c_int = 9;
const SVC_UPDATENAME: c_int = 13;
const SVC_UPDATECOLORS: c_int = 17;
/// `protocol.h:407-408`
const EFLAGS_VIEWMODEL: u8 = 4;
const EFLAGS_EXTERIORMODEL: u8 = 8;
/// `protocol.h` -- FTE pext2 bit read by `SV_UpdateInfo`.
const PEXT2_PREDINFO: c_uint = 0x20;
/// `server.h:288-295` -- `efx_t`.
const EF_BRIGHTFIELD: c_int = 1;
const EF_MUZZLEFLASH: c_int = 2;
const EF_BRIGHTLIGHT: c_int = 4;
const EF_DIMLIGHT: c_int = 8;
const EF_QEX_QUADLIGHT: c_int = 16;
const EF_QEX_PENTALIGHT: c_int = 32;
/// `gl_model.h:595-602` -- model flags.
const MDLF_ROCKET: i32 = 1;
const MDLF_GRENADE: i32 = 2;
const MDLF_GIB: i32 = 4;
const MDLF_ROTATE: i32 = 8;
const MDLF_TRACER: i32 = 16;
const MDLF_ZOMGIB: i32 = 32;
const MDLF_TRACER2: i32 = 64;
const MDLF_TRACER3: i32 = 128;
/// `gl_model.h:610-611`
const MOD_EMITREPLACE: i32 = 2048;
const MOD_EMITFORWARDS: i32 = 4096;
/// `gl_model.h:588-592` -- `modtype_t`.
const MOD_ALIAS: i32 = 2;
/// `mathlib.h`
const PITCH: usize = 0;
const YAW: usize = 1;
const ROLL: usize = 2;

/// `cl_main_glue.c` write-op kinds.
const W_BYTE: c_int = 0;
const W_STRING: c_int = 1;

/// How many `MSG_Write*` ops are buffered before a flush. `cl_main.c`'s
/// longest uninterrupted run is four (`CL_SignonReply` case 2), so this is
/// only a headroom figure -- the emitted bytes do not depend on it.
const WRITE_BATCH: usize = 16;

// ---------------------------------------------------------------------------
// Mirrors and externs this module needs on top of `quake-c-sys`.

extern "C" {
    /// `Quake/protocol.c` -- the all-zero baseline `entity_state_t`.
    static nullentitystate: EntityState;
    /// `Quake/cl_tent_glue.c` (`cl_tent.c:27`).
    static mut cl_temp_entities: [EntityOpaque; MAX_TEMP_ENTITIES];
    /// `Quake/gl_rmain.c` -- read by `CL_Tracepos_f`/`CL_Viewpos_f`.
    static mut r_refdef: RefDef;
}

// ---------------------------------------------------------------------------
// helpers

fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl)
}

fn cls_p() -> *mut ClientStatic {
    ptr::addr_of_mut!(cls)
}

/// `q_minmax.h:50` -- `clamp_i`.
fn clamp_i(minval: c_int, val: c_int, maxval: c_int) -> c_int {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

/// `q_minmax.h:46` -- `q_max_i`.
fn q_max_i(a: c_int, b: c_int) -> c_int {
    if a > b {
        a
    } else {
        b
    }
}

/// `q_minmax.h:46` -- `q_max_d`. `cl_main.c:643` mixes a `float` with the
/// `double` `MAX_PHYSICS_FREQ`, so the comparison happens in `double`.
/// COMPAT: ADR-010 -- the promotion point is reproduced, not simplified.
fn q_max_d(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

/// `&cl.entities[i]`, striding by the authoritative opaque `entity_t` size.
///
/// # Safety
/// `cl.entities` must be non-null and `i` in range.
unsafe fn ent_at(i: c_int) -> *mut Entity {
    // SAFETY: the caller's contract.
    unsafe { (*cl_p()).entities.offset(i as isize).cast::<Entity>() }
}

/// `&cl.scores[i]`.
///
/// # Safety
/// `cl.scores` must be non-null and `i` in range.
unsafe fn score_at(i: c_int) -> *mut ScoreBoard {
    // SAFETY: the caller's contract.
    unsafe { (*cl_p()).scores.offset(i as isize) }
}

/// A buffered run of `MSG_Write*` ops, replayed by one guarded C frame.
///
/// Every `MSG_Write*` reaches `SZ_GetSpace` (`net_msg.c:481`), which
/// `Host_Error`s on overflow, so no Rust frame may sit under one (ADR-009).
/// The ops replay in insertion order, so the emitted byte stream is identical
/// to the C original's.
struct Writer {
    sb: *mut c_void,
    ops: [c::cl_main::ClMainWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new(sb: *mut c_void) -> Self {
        Writer {
            sb,
            ops: [c::cl_main::ClMainWriteOp {
                kind: 0,
                i: 0,
                p: ptr::null(),
            }; WRITE_BATCH],
            n: 0,
        }
    }

    fn flush(&mut self) -> Raise {
        if self.n == 0 {
            return 0;
        }
        let n = self.n;
        self.n = 0;
        // SAFETY: `sb` is a live `sizebuf_t` and the first `n` ops are
        // initialised; the callee only reads them.
        unsafe { g::ClMain_Glue_WriteBatch(self.sb, self.ops.as_ptr(), n as c_int) }
    }

    fn push(&mut self, kind: c_int, i: c_int, p: *const c_void) -> Raise {
        if self.n == WRITE_BATCH {
            raise!(self.flush());
        }
        self.ops[self.n] = c::cl_main::ClMainWriteOp { kind, i, p };
        self.n += 1;
        0
    }

    fn byte(&mut self, v: c_int) -> Raise {
        self.push(W_BYTE, v, ptr::null())
    }

    fn string(&mut self, s: *const c_char) -> Raise {
        self.push(W_STRING, 0, s.cast())
    }
}

// ---------------------------------------------------------------------------
// cl_main.c:76 -- CL_ClearTrailStates

unsafe fn cl_clear_trail_states() -> Raise {
    // SAFETY: `cl` is initialised; the static/entity arrays are sized by the
    // counters read here, and `cl_beams` is a fixed C array.
    unsafe {
        for i in 0..(*cl_p()).num_statics {
            let se = (*(*cl_p()).static_entities.offset(i as isize)).cast::<Entity>();
            raise!(g::ClMain_Glue_DelinkTrailstate(
                ptr::addr_of_mut!((*se).trailstate).cast()
            ));
            raise!(g::ClMain_Glue_DelinkTrailstate(
                ptr::addr_of_mut!((*se).emitstate).cast()
            ));
        }
        for i in 0..(*cl_p()).max_edicts {
            let e = ent_at(i);
            raise!(g::ClMain_Glue_DelinkTrailstate(
                ptr::addr_of_mut!((*e).trailstate).cast()
            ));
            raise!(g::ClMain_Glue_DelinkTrailstate(
                ptr::addr_of_mut!((*e).emitstate).cast()
            ));
        }
        let beams = ptr::addr_of_mut!(c::cl_tent::cl_beams).cast::<c::cl_tent::beam_t>();
        for i in 0..MAX_BEAMS {
            raise!(g::ClMain_Glue_DelinkTrailstate(
                ptr::addr_of_mut!((*beams.add(i)).trailstate).cast()
            ));
        }
    }
    0
}

/// `cl_main.c:76`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_clear_trail_states() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_clear_trail_states() }
}

// ---------------------------------------------------------------------------
// cl_main.c:96 -- CL_FreeState

unsafe fn cl_free_state() -> Raise {
    // SAFETY: `cl`'s pointers are either null or engine allocations; `Mem_Free`
    // tolerates null, exactly as the C original relies on.
    unsafe {
        for i in 0..MAX_CL_STATS {
            c::Mem_Free((*cl_p()).statss[i].cast());
        }
        raise!(g::ClMain_Glue_PRClearProgs(
            ptr::addr_of_mut!((*cl_p()).qcvm).cast()
        ));
        if !(*cl_p()).entities.is_null() {
            for i in 0..(*cl_p()).max_edicts {
                raise!(g::ClMain_Glue_FreeEntityBLAS(ent_at(i).cast()));
            }
        }
        c::Mem_Free((*cl_p()).entities.cast());
        for i in 0..(*cl_p()).num_statics {
            raise!(g::ClMain_Glue_FreeEntityBLAS(
                (*(*cl_p()).static_entities.offset(i as isize)).cast()
            ));
        }
        let mut i = 0;
        while i < (*cl_p()).num_statics {
            c::Mem_Free((*(*cl_p()).static_entities.offset(i as isize)).cast());
            i += 64;
        }
        c::Mem_Free((*cl_p()).static_entities.cast());
        c::Mem_Free((*cl_p()).scores.cast());
        for i in 0..MAX_PARTICLETYPES {
            c::Mem_Free((*cl_p()).particle_precache[i].name.cast());
        }
        for i in 0..(*cl_p()).num_efragallocs {
            c::Mem_Free((*(*cl_p()).efrag_allocs.offset(i as isize)).cast());
        }
        c::Mem_Free((*cl_p()).efrag_allocs.cast());
        ptr::write_bytes(cl_p().cast::<u8>(), 0, core::mem::size_of::<ClientState>());
    }
    0
}

/// `cl_main.c:96`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_free_state() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_free_state() }
}

// ---------------------------------------------------------------------------
// cl_main.c:129 -- CL_ClearState

unsafe fn cl_clear_state() -> Raise {
    // SAFETY: the arrays below are fixed C objects and `cl` is initialised.
    unsafe {
        if !crate::sv_main::sv.active {
            raise!(g::ClMain_Glue_HostClearMemory());
        }

        raise!(cl_free_state());

        c::cvar_cmd::SZ_Clear(ptr::addr_of_mut!((*cls_p()).message).cast());

        ptr::write_bytes(
            ptr::addr_of_mut!(g::cl_dlights).cast::<u8>(),
            0,
            core::mem::size_of::<[c::cl_tent::dlight_t; MAX_DLIGHTS]>(),
        );
        ptr::write_bytes(
            ptr::addr_of_mut!(c::cl_parse::cl_lightstyle).cast::<u8>(),
            0,
            core::mem::size_of::<[c::cl_parse::lightstyle_t; MAX_LIGHTSTYLES]>(),
        );
        ptr::write_bytes(
            ptr::addr_of_mut!(cl_temp_entities).cast::<u8>(),
            0,
            core::mem::size_of::<[EntityOpaque; MAX_TEMP_ENTITIES]>(),
        );
        ptr::write_bytes(
            ptr::addr_of_mut!(c::cl_tent::cl_beams).cast::<u8>(),
            0,
            core::mem::size_of::<[c::cl_tent::beam_t; MAX_BEAMS]>(),
        );

        (*cl_p()).max_edicts = clamp_i(
            MIN_EDICTS,
            c::sv_main::max_edicts.value as c_int,
            MAX_EDICTS,
        );
        (*cl_p()).entities =
            c::Mem_Alloc((*cl_p()).max_edicts as usize * core::mem::size_of::<EntityOpaque>())
                .cast::<EntityOpaque>();

        let viewent = ptr::addr_of_mut!((*cl_p()).viewent).cast::<Entity>();
        (*viewent).netstate = ptr::read(ptr::addr_of!(nullentitystate));

        raise!(g::ClMain_Glue_PScriptShutdown());
    }
    0
}

/// `cl_main.c:129`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_clear_state() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_clear_state() }
}

// ---------------------------------------------------------------------------
// cl_main.c:165 -- CL_Disconnect

unsafe fn cl_disconnect() -> Raise {
    // SAFETY: `cl`/`cls` are initialised; every callee below is either a
    // guarded trampoline or a non-raising engine entry point.
    unsafe {
        if c::cl_demo::key_dest == KEY_MESSAGE {
            raise!(g::ClMain_Glue_KeyEndChat());
        }

        raise!(g::ClMain_Glue_StopAudio());

        if (*cls_p()).demoplayback {
            crate::cl_demo::quake_rs_cl_stop_playback();
        } else if (*cls_p()).state == CA_CONNECTED {
            if (*cls_p()).demorecording {
                raise!(crate::cl_demo::quake_rs_cl_stop_f());
            }

            c::Con_DPrintf(c"Sending clc_disconnect\n".as_ptr());
            let msg = ptr::addr_of_mut!((*cls_p()).message);
            c::cvar_cmd::SZ_Clear(msg.cast());
            let mut w = Writer::new(msg.cast());
            raise!(w.byte(CLC_DISCONNECT));
            raise!(w.flush());
            let mut ignored: c_int = 0;
            raise!(g::ClMain_Glue_NetSendUnreliable(
                (*cls_p()).netcon.cast(),
                msg.cast(),
                ptr::addr_of_mut!(ignored)
            ));
            c::cvar_cmd::SZ_Clear(msg.cast());
            raise!(g::ClMain_Glue_NetClose((*cls_p()).netcon.cast()));
            (*cls_p()).netcon = ptr::null_mut();

            (*cls_p()).state = CA_DISCONNECTED;
            if crate::sv_main::sv.active {
                raise!(g::ClMain_Glue_HostShutdownServer(0));
            }
        }

        (*cls_p()).timedemo = false;
        (*cls_p()).demoplayback = false;
        (*cls_p()).demopaused = false;
        (*cls_p()).signon = 0;
        (*cls_p()).netcon = ptr::null_mut();
        (*cl_p()).intermission = 0;
        (*cl_p()).worldmodel = ptr::null_mut();
        (*cl_p()).sendprespawn = false;
        raise!(g::ClMain_Glue_CenterPrintClear());
    }
    0
}

/// `cl_main.c:165`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_disconnect() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_disconnect() }
}

// ---------------------------------------------------------------------------
// cl_main.c:206 -- CL_Disconnect_f

/// `cl_main.c:206`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_disconnect_f() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe {
        raise!(cl_disconnect());
        if crate::sv_main::sv.active {
            raise!(g::ClMain_Glue_HostShutdownServer(0));
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:220 -- CL_EstablishConnection

unsafe fn cl_establish_connection(host: *const c_char) -> Raise {
    // SAFETY: `host` is a NUL-terminated string owned by the caller.
    unsafe {
        if (*cls_p()).state == CA_DEDICATED {
            return 0;
        }

        if (*cls_p()).demoplayback {
            return 0;
        }

        raise!(cl_disconnect());

        let mut sock: *mut c::qsocket_s = ptr::null_mut();
        raise!(g::ClMain_Glue_NetConnect(host, ptr::addr_of_mut!(sock)));
        (*cls_p()).netcon = sock.cast();
        if (*cls_p()).netcon.is_null() {
            return RAISE_CONNECT_FAILED;
        }
        c::Con_DPrintf(c"CL_EstablishConnection: connected to %s\n".as_ptr(), host);

        (*cls_p()).demonum = -1;
        (*cls_p()).state = CA_CONNECTED;
        (*cls_p()).signon = 0;
        let mut w = Writer::new(ptr::addr_of_mut!((*cls_p()).message).cast());
        raise!(w.byte(CLC_NOP));
        raise!(w.flush());
    }
    0
}

/// `cl_main.c:220`.
///
/// # Safety
/// `host` must be a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_establish_connection(host: *const c_char) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { cl_establish_connection(host) }
}

// ---------------------------------------------------------------------------
// cl_main.c:241 -- CL_SendInitialUserinfo

unsafe fn cl_send_initial_userinfo(key: *const c_char, val: *const c_char) -> Raise {
    // SAFETY: `key`/`val` are NUL-terminated strings supplied by
    // `Info_Enumerate`.
    unsafe {
        if *key == b'*' as c_char {
            return 0;
        }
        if c::cl_main::strcmp(key, c"name".as_ptr()) == 0 {
            return 0;
        }
        let mut w = Writer::new(ptr::addr_of_mut!((*cls_p()).message).cast());
        raise!(w.byte(CLC_STRINGCMD));
        raise!(w.string(c::cl_main::va(
            c"setinfo \"%s\" \"%s\"\n".as_ptr(),
            key,
            val
        )));
        raise!(w.flush());
    }
    0
}

/// `cl_main.c:241`. The `ctx` operand is `Info_Enumerate`'s opaque cookie;
/// the C original ignores it too.
///
/// # Safety
/// `key` and `val` must be valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_send_initial_userinfo(
    _ctx: *mut c_void,
    key: *const c_char,
    val: *const c_char,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { cl_send_initial_userinfo(key, val) }
}

// ---------------------------------------------------------------------------
// cl_main.c:257 -- CL_SignonReply

unsafe fn cl_signon_reply() -> Raise {
    let mut str_ = [0 as c_char; 8192];
    // SAFETY: `cl`/`cls` are initialised; `str_` is a live local.
    unsafe {
        c::Con_DPrintf(c"CL_SignonReply: %i\n".as_ptr(), (*cls_p()).signon);

        let msg = ptr::addr_of_mut!((*cls_p()).message).cast();
        match (*cls_p()).signon {
            1 => {
                let mut w = Writer::new(msg);
                raise!(w.byte(CLC_STRINGCMD));
                raise!(w.string(c::cl_main::va(c"name \"%s\"\n".as_ptr(), g::cl_name.string)));
                raise!(w.flush());

                (*cl_p()).sendprespawn = true;
            }
            2 => {
                let mut w = Writer::new(msg);
                raise!(w.byte(CLC_STRINGCMD));
                raise!(w.string(c::cl_main::va(
                    c"color %i %i\n".as_ptr(),
                    g::cl_topcolor.value as c_int,
                    g::cl_bottomcolor.value as c_int
                )));
                raise!(w.flush());

                if (*cl_p()).serverinfo[0] != 0 {
                    raise!(g::ClMain_Glue_InfoEnumerate((*cls_p()).userinfo.as_ptr()));
                }

                raise!(w.byte(CLC_STRINGCMD));
                c::cl_main::q_snprintf(
                    str_.as_mut_ptr(),
                    str_.len(),
                    c"spawn %s".as_ptr(),
                    (*cls_p()).spawnparms.as_ptr(),
                );
                raise!(w.string(str_.as_ptr()));
                raise!(w.flush());
            }
            3 => {
                let mut w = Writer::new(msg);
                raise!(w.byte(CLC_STRINGCMD));
                raise!(w.string(c"begin".as_ptr()));
                raise!(w.flush());
            }
            4 => {
                raise!(g::ClMain_Glue_EndLoadingPlaque());
            }
            _ => {}
        }
    }
    0
}

/// `cl_main.c:257`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_signon_reply() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_signon_reply() }
}

// ---------------------------------------------------------------------------
// cl_main.c:302 -- CL_NextDemo

unsafe fn cl_next_demo() -> Raise {
    let mut str_ = [0 as c_char; 1024];
    // SAFETY: `cls.demos` is a fixed C array and `cls.demonum` is bounds-
    // checked exactly as the C original does.
    unsafe {
        if (*cls_p()).demonum == -1 {
            return 0;
        }

        if (*cls_p()).demos[(*cls_p()).demonum as usize][0] == 0 || (*cls_p()).demonum == MAX_DEMOS
        {
            (*cls_p()).demonum = 0;
            if (*cls_p()).demos[(*cls_p()).demonum as usize][0] == 0 {
                c::Con_Printf(c"No demos listed with startdemos\n".as_ptr());
                (*cls_p()).demonum = -1;
                raise!(cl_disconnect());
                return 0;
            }
        }

        raise!(g::ClMain_Glue_BeginLoadingPlaque());

        c::cl_main::q_snprintf(
            str_.as_mut_ptr(),
            str_.len(),
            c"playdemo %s\n".as_ptr(),
            (*cls_p()).demos[(*cls_p()).demonum as usize].as_ptr(),
        );
        raise!(g::ClMain_Glue_CbufInsertText(str_.as_ptr()));
        (*cls_p()).demonum += 1;
    }
    0
}

/// `cl_main.c:302`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_next_demo() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_next_demo() }
}

// ---------------------------------------------------------------------------
// cl_main.c:333 -- CL_PrintEntities_f

/// `cl_main.c:333`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_print_entities_f() -> c_int {
    // SAFETY: `cl.entities` holds `cl.num_entities` live entities whenever
    // `cls.state == ca_connected`.
    unsafe {
        if (*cls_p()).state != CA_CONNECTED {
            return 0;
        }

        for i in 0..(*cl_p()).num_entities {
            let ent = ent_at(i);
            c::Con_Printf(c"%3i:".as_ptr(), i);
            if (*ent).model.is_null() {
                c::Con_Printf(c"EMPTY\n".as_ptr());
                continue;
            }
            c::Con_Printf(
                c"%s:%2i  (%5.1f,%5.1f,%5.1f) [%5.1f %5.1f %5.1f]\n".as_ptr(),
                (*(*ent).model).name.as_ptr(),
                (*ent).frame,
                (*ent).origin[0] as f64,
                (*ent).origin[1] as f64,
                (*ent).origin[2] as f64,
                (*ent).angles[0] as f64,
                (*ent).angles[1] as f64,
                (*ent).angles[2] as f64,
            );
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:361 -- CL_AllocDlight

/// Fills a freshly claimed dlight the way `cl_main.c:373-377` does.
///
/// # Safety
/// `dl` must point at a live `dlight_t`.
unsafe fn dlight_init(dl: *mut c::cl_tent::dlight_t, key: c_int, kex: bool) {
    // SAFETY: the caller's contract.
    unsafe {
        ptr::write_bytes(
            dl.cast::<u8>(),
            0,
            core::mem::size_of::<c::cl_tent::dlight_t>(),
        );
        (*dl).key = key;
        (*dl).color[0] = 1.0;
        (*dl).color[1] = 1.0;
        (*dl).color[2] = 1.0;
        (*dl).cone_cos = -2.0;
        if kex {
            (*dl).kex_intensity = 0.0;
        }
    }
}

/// `cl_main.c:361`. Cannot raise; the glue calls it directly.
#[no_mangle]
pub extern "C" fn quake_rs_cl_alloc_dlight(key: c_int) -> *mut c_void {
    // SAFETY: `cl_dlights` is a fixed C array of `MAX_DLIGHTS`.
    unsafe {
        let base = ptr::addr_of_mut!(g::cl_dlights).cast::<c::cl_tent::dlight_t>();

        if key != 0 {
            for i in 0..MAX_DLIGHTS {
                let dl = base.add(i);
                if (*dl).key == key {
                    dlight_init(dl, key, true);
                    return dl.cast();
                }
            }
        }

        for i in 0..MAX_DLIGHTS {
            let dl = base.add(i);
            if ((*dl).die as f64) < (*cl_p()).time {
                dlight_init(dl, key, true);
                return dl.cast();
            }
        }

        // cl_main.c:405 -- note the fallback slot does NOT reset
        // kex_intensity; the divergence from the two loops above is in the C.
        let dl = base;
        dlight_init(dl, key, false);
        dl.cast()
    }
}

// ---------------------------------------------------------------------------
// cl_main.c:413 -- CL_DecayLights

/// `cl_main.c:413`. Cannot raise.
#[no_mangle]
pub extern "C" fn quake_rs_cl_decay_lights() {
    // SAFETY: `cl_dlights` is a fixed C array of `MAX_DLIGHTS`.
    unsafe {
        // COMPAT: ADR-010 -- `cl.time`/`cl.oldtime` are `double`, `time` is
        // `float`; the subtraction happens in `double` and narrows on store.
        let time = ((*cl_p()).time - (*cl_p()).oldtime) as c_float;
        if time < 0.0 {
            return;
        }

        let base = ptr::addr_of_mut!(g::cl_dlights).cast::<c::cl_tent::dlight_t>();
        for i in 0..MAX_DLIGHTS {
            let dl = base.add(i);
            if ((*dl).die as f64) < (*cl_p()).time || (*dl).radius == 0.0 {
                continue;
            }

            (*dl).radius -= time * (*dl).decay;
            if (*dl).radius < 0.0 {
                (*dl).radius = 0.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// cl_main.c:443 -- CL_LerpPoint

/// `cl_main.c:443`. Cannot raise.
///
/// COMPAT: ADR-010 -- `f` and `frac` are C `float` locals fed entirely by
/// `double` expressions, so each assignment narrows. Reproduced literally.
#[no_mangle]
pub extern "C" fn quake_rs_cl_lerp_point() -> c_float {
    // SAFETY: `cl`/`cls` are initialised.
    unsafe {
        let mut f: c_float = ((*cl_p()).mtime[0] - (*cl_p()).mtime[1]) as c_float;

        if f == 0.0
            || (*cls_p()).timedemo
            || (crate::sv_main::sv.active && g::host_netinterval == 0.0)
        {
            (*cl_p()).time = (*cl_p()).mtime[0];
            return 1.0;
        }

        if f as f64 > 0.1 {
            (*cl_p()).mtime[1] = (*cl_p()).mtime[0] - 0.1;
            f = 0.1;
        }

        let mut frac: c_float = (((*cl_p()).time - (*cl_p()).mtime[1]) / f as f64) as c_float;

        if frac < 0.0 {
            if (frac as f64) < -0.01 {
                (*cl_p()).time = (*cl_p()).mtime[1];
            }
            frac = 0.0;
        } else if frac > 1.0 {
            if frac as f64 > 1.01 {
                (*cl_p()).time = (*cl_p()).mtime[0];
            }
            frac = 1.0;
        }

        if g::cl_nolerp.value != 0.0 {
            return 1.0;
        }

        frac
    }
}

// ---------------------------------------------------------------------------
// cl_main.c:484 -- CL_LerpEntity (static)

/// # Safety
/// `ent` must point at a live `entity_t`; `org`/`ang` at three writable floats.
unsafe fn cl_lerp_entity(
    ent: *mut Entity,
    org: *mut [c_float; 3],
    ang: *mut [c_float; 3],
    frac: c_float,
) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let mut delta = [0.0f32; 3];
        let mut teleported = false;

        if (*ent).forcelink {
            *org = (*ent).msg_origins[0];
            *ang = (*ent).msg_angles[0];
        } else {
            let mut f = frac;
            #[allow(clippy::needless_range_loop)] // indexes msg_origins too
            for j in 0..3 {
                delta[j] = (*ent).msg_origins[0][j] - (*ent).msg_origins[1][j];
                if delta[j] > 100.0 || delta[j] < -100.0 {
                    f = 1.0;
                    teleported = true;
                }
            }

            let mut a = f;

            if c::sv_send::r_lerpmove.value != 0.0 && (*ent).lerp.movestep {
                f = 1.0;

                if g::r_lerpturn.value != 0.0 {
                    a = 1.0;
                }
            }

            #[allow(clippy::needless_range_loop)] // five parallel arrays
            for j in 0..3 {
                (*org)[j] = (*ent).msg_origins[1][j] + f * delta[j];

                let mut d = (*ent).msg_angles[0][j] - (*ent).msg_angles[1][j];
                if d > 180.0 {
                    d -= 360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                (*ang)[j] = (*ent).msg_angles[1][j] + a * d;
            }
        }
        teleported
    }
}

// ---------------------------------------------------------------------------
// cl_main.c:539 -- CL_AttachEntity (static)

/// # Safety
/// `ent` must point at a live `entity_t` and `cl.entities` be populated.
unsafe fn cl_attach_entity(ent: *mut Entity, frac: c_float) -> bool {
    // SAFETY: the caller's contract; `tagent` is bounds-checked below exactly
    // as the C original does.
    unsafe {
        let mut tagent: c_uint = (*ent).netstate.tagentity as c_uint;
        let mut runaway = 0;

        loop {
            if tagent == 0 {
                return true;
            }
            let r = runaway;
            runaway += 1;
            if r == 10 || tagent >= (*cl_p()).num_entities as c_uint {
                return false;
            }
            let parent = ent_at(tagent as c_int);

            if tagent == (*cl_p()).viewentity as c_uint {
                (*ent).eflags |= EFLAGS_EXTERIORMODEL;
            }

            if (*parent).model.is_null() {
                return false;
            }

            let mut porg = [0.0f32; 3];
            let mut pang = [0.0f32; 3];
            tagent = (*parent).netstate.tagentity as c_uint;
            cl_lerp_entity(
                parent,
                ptr::addr_of_mut!(porg),
                ptr::addr_of_mut!(pang),
                frac,
            );

            if !(*parent).model.is_null() && (*(*parent).model).type_ == MOD_ALIAS {
                pang[0] *= -1.0;
            }
            let mut paxis = [[0.0f32; 3]; 3];
            let (mut p0, mut p1, mut p2) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
            m::angle_vectors(&pang, &mut p0, &mut p1, &mut p2);
            paxis[0] = p0;
            paxis[1] = p1;
            paxis[2] = p2;

            if !(*ent).model.is_null() && (*(*ent).model).type_ == MOD_ALIAS {
                (*ent).angles[0] *= -1.0;
            }
            let (mut fwd, mut tmp, mut up) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
            m::angle_vectors(&(*ent).angles, &mut fwd, &mut tmp, &mut up);

            let mut out = [0.0f32; 3];
            m::vector_ma(&(*parent).origin, (*ent).origin[0], &paxis[0], &mut out);
            tmp = out;
            m::vector_ma(&tmp, -(*ent).origin[1], &paxis[1], &mut out);
            tmp = out;
            m::vector_ma(&tmp, (*ent).origin[2], &paxis[2], &mut out);
            (*ent).origin = out;

            m::vector_ma(&m::VEC3_ORIGIN, fwd[0], &paxis[0], &mut out);
            tmp = out;
            m::vector_ma(&tmp, -fwd[1], &paxis[1], &mut out);
            tmp = out;
            m::vector_ma(&tmp, fwd[2], &paxis[2], &mut out);
            fwd = out;

            m::vector_ma(&m::VEC3_ORIGIN, up[0], &paxis[0], &mut out);
            tmp = out;
            m::vector_ma(&tmp, -up[1], &paxis[1], &mut out);
            tmp = out;
            m::vector_ma(&tmp, up[2], &paxis[2], &mut out);
            up = out;

            let mut angles = [0.0f32; 3];
            m::vector_angles(&fwd, Some(&up), &mut angles);
            (*ent).angles = angles;
            if !(*ent).model.is_null() && (*(*ent).model).type_ == MOD_ALIAS {
                (*ent).angles[0] *= -1.0;
            }

            (*ent).eflags |= (*parent).netstate.eflags & (EFLAGS_VIEWMODEL | EFLAGS_EXTERIORMODEL);
        }
    }
}

// ---------------------------------------------------------------------------
// cl_main.c:617 -- CL_ResetTrail / cl_main.c:630 -- CL_RocketTrail (static)

/// # Safety
/// `ent` must point at a live `entity_t`.
unsafe fn cl_reset_trail(ent: *mut Entity) {
    // SAFETY: the caller's contract.
    unsafe {
        // COMPAT: ADR-010 -- `1.f / MAX_PHYSICS_FREQ` is float / double, so
        // the quotient is computed in `double` and narrowed on store.
        (*ent).traildelay = (1.0f32 as f64 / MAX_PHYSICS_FREQ) as c_float;
        (*ent).trailorg = (*ent).origin;
    }
}

/// # Safety
/// `ent` must point at a live `entity_t`.
unsafe fn cl_rocket_trail(ent: *mut Entity, type_: c_int) -> Raise {
    // SAFETY: the caller's contract.
    unsafe {
        // COMPAT: ADR-010 -- `float -= double` computes in `double`.
        (*ent).traildelay =
            ((*ent).traildelay as f64 - ((*cl_p()).time - (*cl_p()).oldtime)) as c_float;
        if (*ent).traildelay > 0.0 {
            return 0;
        }
        raise!(g::ClMain_Glue_RocketTrail(
            (*ent).trailorg.as_ptr(),
            (*ent).origin.as_ptr(),
            type_
        ));

        (*ent).traildelay = q_max_d(
            0.0f32 as f64,
            (*ent).traildelay as f64 + 1.0f32 as f64 / MAX_PHYSICS_FREQ,
        ) as c_float;
        (*ent).trailorg = (*ent).origin;
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:646 -- CL_RelinkEntities

unsafe fn cl_relink_entities() -> Raise {
    // SAFETY: `cl` is initialised; every entity index below is bounded by
    // `cl.num_entities`, and the visedicts arrays are grown first.
    unsafe {
        let frac = quake_rs_cl_lerp_point();

        let mut frametime = ((*cl_p()).time - (*cl_p()).oldtime) as c_float;
        if frametime < 0.0 {
            frametime = 0.0;
        }
        if frametime as f64 > 0.1 {
            frametime = 0.1;
        }

        if g::cl_numvisedicts + 256 > g::cl_maxvisedicts {
            g::cl_maxvisedicts += if g::cl_maxvisedicts != 0 { 256 } else { 4096 };
            g::cl_visedicts = c::Mem_Realloc(
                g::cl_visedicts.cast(),
                core::mem::size_of::<*mut c_void>() * g::cl_maxvisedicts as usize,
            )
            .cast();
            g::cl_visedicts_alpha = c::Mem_Realloc(
                g::cl_visedicts_alpha.cast(),
                core::mem::size_of::<*mut c_void>() * g::cl_maxvisedicts as usize,
            )
            .cast();
        }
        g::cl_numvisedicts = 0;

        for i in 0..3 {
            (*cl_p()).velocity[i] = (*cl_p()).mvelocity[1][i]
                + frac * ((*cl_p()).mvelocity[0][i] - (*cl_p()).mvelocity[1][i]);
        }

        raise!(g::ClMain_Glue_UpdateZoom());

        if (*cls_p()).demoplayback {
            for j in 0..3 {
                let mut d = (*cl_p()).mviewangles[0][j] - (*cl_p()).mviewangles[1][j];
                if d > 180.0 {
                    d -= 360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                (*cl_p()).viewangles[j] = (*cl_p()).mviewangles[1][j] + frac * d;
            }
        }

        // COMPAT: ADR-010 -- `100 * cl.time` is `int * double`, so `anglemod`
        // receives the narrowed `double` product.
        let bobjrotate = m::anglemod((100.0 * (*cl_p()).time) as c_float);

        // cl_main.c:698-700 -- `ent = cl.entities ? cl.entities + 1 : NULL`
        // then a plain walk. `cl.entities` is only NULL when `cl` was just
        // memset, in which case `num_entities` is 0 and the body never runs,
        // so the pointer arithmetic below matches the C exactly.
        for i in 1..(*cl_p()).num_entities {
            let ent = ent_at(i);
            if (*ent).model.is_null() {
                continue;
            }
            (*ent).eflags = (*ent).netstate.eflags;

            if (*ent).msgtime != (*cl_p()).mtime[0] {
                (*ent).model = ptr::null_mut();
                raise!(g::ClMain_Glue_FreeEntityBLAS(ent.cast()));
                raise!(g::ClMain_Glue_InvalidateTraceLineCache());
                continue;
            }

            let oldorg = (*ent).origin;

            let teleported = cl_lerp_entity(
                ent,
                ptr::addr_of_mut!((*ent).origin),
                ptr::addr_of_mut!((*ent).angles),
                frac,
            );

            if (*cl_p()).time < (*cl_p()).oldtime {
                (*ent).lerp.prev_frame = (*ent).frame;
                (*ent).lerp.frame_change_time = 0.0;
                (*ent).lerp.snap_frames = 0;
                (*ent).lerp.prev_origin = (*ent).msg_origins[0];
                (*ent).lerp.prev_angles = (*ent).msg_angles[0];
                (*ent).lerp.move_change_time = 0.0;
            }

            if (*ent).netstate.tagentity != 0 && !cl_attach_entity(ent, frac) {
                continue;
            }

            let mut modelflags = ((*ent).effects >> 24) & 0xff;
            modelflags |= (*(*ent).model).flags;

            if (*ent).forcelink || teleported {
                cl_reset_trail(ent);
            }

            if modelflags & MDLF_ROTATE != 0 {
                (*ent).angles[1] = bobjrotate;
            }

            if (*ent).effects & EF_BRIGHTFIELD != 0 {
                raise!(g::ClMain_Glue_EntityParticles(ent.cast()));
            }

            if (*ent).effects & EF_MUZZLEFLASH != 0 {
                let dl = quake_rs_cl_alloc_dlight(i).cast::<c::cl_tent::dlight_t>();
                (*dl).origin = (*ent).origin;
                (*dl).origin[2] += 16.0;
                let (mut fv, mut rv, mut uv) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
                m::angle_vectors(&(*ent).angles, &mut fv, &mut rv, &mut uv);

                let mut out = [0.0f32; 3];
                m::vector_ma(&(*dl).origin, 18.0, &fv, &mut out);
                (*dl).origin = out;
                (*dl).radius = (200 + (c::COM_Rand() & 31)) as c_float;
                (*dl).minlight = 32.0;
                (*dl).die = ((*cl_p()).time + 0.1) as c_float;

                if g::r_lerpmodels.value != 2.0 {
                    let viewent = ptr::addr_of_mut!((*cl_p()).viewent).cast::<Entity>();
                    if ent == ent_at((*cl_p()).viewentity) {
                        if (*viewent).lerp.snap_msgtime != (*ent).msgtime {
                            (*viewent).lerp.prev_frame = (*viewent).frame;
                            (*viewent).lerp.frame_change_time = 0.0;
                            (*viewent).lerp.snap_frames = 2;
                            (*viewent).lerp.snap_msgtime = (*ent).msgtime;
                        }
                    } else {
                        (*ent).lerp.prev_frame = (*ent).frame;
                        (*ent).lerp.frame_change_time = 0.0;
                        (*ent).lerp.snap_frames = 1;
                    }
                }
            }
            if (*ent).effects & EF_BRIGHTLIGHT != 0 {
                let dl = quake_rs_cl_alloc_dlight(i).cast::<c::cl_tent::dlight_t>();
                (*dl).origin = (*ent).origin;
                (*dl).origin[2] += 16.0;
                (*dl).radius = (400 + (c::COM_Rand() & 31)) as c_float;
                (*dl).die = ((*cl_p()).time + 0.001) as c_float;
            }
            if (*ent).effects & EF_DIMLIGHT != 0 {
                let dl = quake_rs_cl_alloc_dlight(i).cast::<c::cl_tent::dlight_t>();
                (*dl).origin = (*ent).origin;
                (*dl).radius = (200 + (c::COM_Rand() & 31)) as c_float;
                (*dl).die = ((*cl_p()).time + 0.001) as c_float;
            }
            if (*ent).effects & EF_QEX_QUADLIGHT != 0 {
                let dl = quake_rs_cl_alloc_dlight(i).cast::<c::cl_tent::dlight_t>();
                (*dl).origin = (*ent).origin;
                (*dl).radius = (200 + (c::COM_Rand() & 31)) as c_float;
                (*dl).die = ((*cl_p()).time + 0.001) as c_float;
                (*dl).color[0] = 0.25;
                (*dl).color[1] = 0.25;
                (*dl).color[2] = 1.0;
            }
            if (*ent).effects & EF_QEX_PENTALIGHT != 0 {
                let dl = quake_rs_cl_alloc_dlight(i).cast::<c::cl_tent::dlight_t>();
                (*dl).origin = (*ent).origin;
                (*dl).radius = (200 + (c::COM_Rand() & 31)) as c_float;
                (*dl).die = ((*cl_p()).time + 0.001) as c_float;
                (*dl).color[0] = 1.0;
                (*dl).color[1] = 0.25;
                (*dl).color[2] = 0.25;
            }

            // `quakedef.h:38` defines PSET_SCRIPT unconditionally, so the
            // `#else` fallback macro at cl_main.c:853 is dead code.
            let tnum = (*ent).netstate.traileffectnum;
            if (*cl_p()).paused {
                // cl_main.c:838 -- deliberately empty.
            } else if tnum > 0 && (tnum as usize) < MAX_PARTICLETYPES {
                let axis = axis_of(&(*ent).angles);
                raise!(g::ClMain_Glue_ParticleTrail(
                    oldorg.as_ptr(),
                    (*ent).origin.as_ptr(),
                    (*cl_p()).particle_precache[tnum as usize].index,
                    frametime,
                    i,
                    axis.as_ptr().cast(),
                    ptr::addr_of_mut!((*ent).trailstate)
                ));
            } else if (*(*ent).model).traileffect >= 0 {
                let axis = axis_of(&(*ent).angles);
                raise!(g::ClMain_Glue_ParticleTrail(
                    oldorg.as_ptr(),
                    (*ent).origin.as_ptr(),
                    (*(*ent).model).traileffect,
                    frametime,
                    i,
                    axis.as_ptr().cast(),
                    ptr::addr_of_mut!((*ent).trailstate)
                ));
            } else {
                let flags = (*(*ent).model).flags;
                let mut out: c_int = 0;
                if flags & MDLF_GIB != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_BLOOD".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 2));
                    }
                } else if flags & MDLF_ZOMGIB != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_SLIGHTBLOOD".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 4));
                    }
                } else if flags & MDLF_TRACER != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_WIZSPIKE".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 3));
                    }
                } else if flags & MDLF_TRACER2 != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_KNIGHTSPIKE".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 5));
                    }
                } else if flags & MDLF_ROCKET != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_ROCKET".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 0));
                    }
                    let dl = quake_rs_cl_alloc_dlight(i).cast::<c::cl_tent::dlight_t>();
                    (*dl).origin = (*ent).origin;
                    (*dl).radius = 200.0;
                    (*dl).die = ((*cl_p()).time + 0.01) as c_float;
                } else if flags & MDLF_GRENADE != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_GRENADE".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 1));
                    }
                } else if flags & MDLF_TRACER3 != 0 {
                    raise!(g::ClMain_Glue_EntParticleTrail(
                        oldorg.as_ptr(),
                        ent.cast(),
                        c"TR_VORESPIKE".as_ptr(),
                        ptr::addr_of_mut!(out)
                    ));
                    if out != 0 {
                        raise!(cl_rocket_trail(ent, 6));
                    }
                }
            }

            (*ent).forcelink = false;

            let enum_ = (*ent).netstate.emiteffectnum;
            if enum_ > 0 {
                let mut axis = axis_of(&(*ent).angles);
                if (*(*ent).model).type_ == MOD_ALIAS {
                    axis[0][2] *= -1.0;
                }
                raise!(g::ClMain_Glue_RunParticleEffectState(
                    (*ent).origin.as_ptr(),
                    axis[0].as_ptr(),
                    frametime,
                    (*cl_p()).particle_precache[enum_ as usize].index,
                    ptr::addr_of_mut!((*ent).emitstate)
                ));
            } else if (*(*ent).model).emiteffect >= 0 {
                let mut axis = axis_of(&(*ent).angles);
                if (*(*ent).model).flags & MOD_EMITFORWARDS != 0 {
                    if (*(*ent).model).type_ == MOD_ALIAS {
                        axis[0][2] *= -1.0;
                    }
                } else {
                    let mut out = [0.0f32; 3];
                    m::vector_scale(&axis[2], -1.0, &mut out);
                    axis[0] = out;
                }
                raise!(g::ClMain_Glue_RunParticleEffectState(
                    (*ent).origin.as_ptr(),
                    axis[0].as_ptr(),
                    frametime,
                    (*(*ent).model).emiteffect,
                    ptr::addr_of_mut!((*ent).emitstate)
                ));
                if (*(*ent).model).flags & MOD_EMITREPLACE != 0 {
                    continue;
                }
            }

            if i == (*cl_p()).viewentity && c::chase::chase_active.value == 0.0 {
                continue;
            }

            if g::cl_numvisedicts < g::cl_maxvisedicts {
                raise!(g::ClMain_Glue_AllocateEntityBLAS(ent.cast()));
                *g::cl_visedicts.offset(g::cl_numvisedicts as isize) = ent.cast();
                g::cl_numvisedicts += 1;
            }
        }

        raise!(g::ClMain_Glue_UpdateEntityDlights());
    }
    0
}

/// `AngleVectors (ent->angles, axis[0], axis[1], axis[2])`.
fn axis_of(angles: &[c_float; 3]) -> [[c_float; 3]; 3] {
    let (mut a0, mut a1, mut a2) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    m::angle_vectors(angles, &mut a0, &mut a1, &mut a2);
    [a0, a1, a2]
}

/// `cl_main.c:646`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_relink_entities() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_relink_entities() }
}

// ---------------------------------------------------------------------------
// cl_main.c:939 -- CL_GenerateRandomParticlePrecache

/// `cl_main.c:939`.
///
/// # Safety
/// `pname` must be a valid NUL-terminated string; `out` writable.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_generate_random_particle_precache(
    pname: *const c_char,
    out: *mut c_int,
) -> c_int {
    // SAFETY: the caller's contract; `cl.particle_precache` is a fixed array.
    unsafe {
        *out = 0;
        // cl_main.c:941 -- the `va` copy is load-bearing: callers pass a
        // pointer into the very array this function writes.
        let pname = c::cl_main::va(c"%s".as_ptr(), pname);
        for i in 1..MAX_PARTICLETYPES {
            if (*cl_p()).particle_precache[i].name.is_null() {
                (*cl_p()).particle_precache[i].name = c::cvar_cmd::q_strdup(pname);
                let mut idx: c_int = 0;
                raise!(g::ClMain_Glue_FindParticleType(
                    (*cl_p()).particle_precache[i].name,
                    ptr::addr_of_mut!(idx)
                ));
                (*cl_p()).particle_precache[i].index = idx;
                *out = i as c_int;
                return 0;
            }
            if c::cl_main::strcmp((*cl_p()).particle_precache[i].name, pname) == 0 {
                *out = i as c_int;
                return 0;
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:965 -- CL_ReadFromServer

unsafe fn cl_read_from_server(out: *mut c_int) -> Raise {
    // SAFETY: `cl`/`cls` are initialised; `out` is writable.
    unsafe {
        *out = 0;

        (*cl_p()).oldtime = (*cl_p()).time;
        (*cl_p()).time += c::host_frametime;

        g::needs_relink = true;
        loop {
            let mut ret: c_int = 0;
            raise!(crate::cl_demo::quake_rs_cl_get_message(ptr::addr_of_mut!(
                ret
            )));
            if ret == -1 {
                return RAISE_LOST_READ;
            }
            if ret == 0 {
                break;
            }

            (*cl_p()).last_received_message = c::cl_demo::realtime as c_float;
            raise!(g::ClMain_Glue_ParseServerMessage());

            if !(ret != 0 && (*cls_p()).state == CA_CONNECTED) {
                break;
            }
        }

        if g::cl_shownet.value != 0.0 {
            c::Con_Printf(c"\n".as_ptr());
        }

        raise!(cl_relink_entities());
        g::needs_relink = false;
        raise!(g::ClMain_Glue_UpdateTEnts());

        if g::cl_numvisedicts > 256 && c::cl_parse::dev_peakstats.visedicts <= 256 {
            c::Con_DWarning(
                c"%i visedicts exceeds standard limit of 256.\n".as_ptr(),
                g::cl_numvisedicts,
            );
        }
        c::cl_parse::dev_stats.visedicts = g::cl_numvisedicts;
        c::cl_parse::dev_peakstats.visedicts =
            q_max_i(g::cl_numvisedicts, c::cl_parse::dev_peakstats.visedicts);

        if c::cl_tent::num_temp_entities > 64 && c::cl_parse::dev_peakstats.tempents <= 64 {
            c::Con_DWarning(
                c"%i tempentities exceeds standard limit of 64 (max = %d).\n".as_ptr(),
                c::cl_tent::num_temp_entities,
                MAX_TEMP_ENTITIES as c_int,
            );
        }
        c::cl_parse::dev_stats.tempents = c::cl_tent::num_temp_entities;
        c::cl_parse::dev_peakstats.tempents = q_max_i(
            c::cl_tent::num_temp_entities,
            c::cl_parse::dev_peakstats.tempents,
        );

        let mut num_beams: c_int = 0;
        let beams = ptr::addr_of!(c::cl_tent::cl_beams).cast::<c::cl_tent::beam_t>();
        for i in 0..MAX_BEAMS {
            let b = beams.add(i);
            if !(*b).model.is_null() && (*b).endtime as f64 >= (*cl_p()).time {
                num_beams += 1;
            }
        }
        if num_beams > 24 && c::cl_parse::dev_peakstats.beams <= 24 {
            c::Con_DWarning(
                c"%i beams exceeded standard limit of 24 (max = %d).\n".as_ptr(),
                num_beams,
                MAX_BEAMS as c_int,
            );
        }
        c::cl_parse::dev_stats.beams = num_beams;
        c::cl_parse::dev_peakstats.beams = q_max_i(num_beams, c::cl_parse::dev_peakstats.beams);

        let mut num_dlights: c_int = 0;
        let dl0 = ptr::addr_of!(g::cl_dlights).cast::<c::cl_tent::dlight_t>();
        for i in 0..MAX_DLIGHTS {
            let l = dl0.add(i);
            if (*l).die as f64 >= (*cl_p()).time && (*l).radius != 0.0 {
                num_dlights += 1;
            }
        }
        if num_dlights > 32 && c::cl_parse::dev_peakstats.dlights <= 32 {
            c::Con_DWarning(
                c"%i dlights exceeded standard limit of 32 (max = %d).\n".as_ptr(),
                num_dlights,
                MAX_DLIGHTS as c_int,
            );
        }
        c::cl_parse::dev_stats.dlights = num_dlights;
        c::cl_parse::dev_peakstats.dlights =
            q_max_i(num_dlights, c::cl_parse::dev_peakstats.dlights);

        *out = 0;
    }
    0
}

/// `cl_main.c:965`.
///
/// # Safety
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_read_from_server(out: *mut c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { cl_read_from_server(out) }
}

// ---------------------------------------------------------------------------
// cl_main.c:1045 -- CL_AccumulateCmd

/// `cl_main.c:1045`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_accumulate_cmd() -> c_int {
    // SAFETY: `cl`/`cls` are initialised.
    unsafe {
        if (*cls_p()).signon == SIGNONS {
            crate::cl_input::CL_AdjustAngles();

            raise!(g::ClMain_Glue_InMove(
                ptr::addr_of_mut!((*cl_p()).pendingcmd).cast()
            ));
        }

        // COMPAT: ADR-010 -- `cl.mtime[0]` is `double` and `seconds` is
        // `float`; the difference is computed in `double` and narrowed.
        (*cl_p()).pendingcmd.seconds =
            ((*cl_p()).mtime[0] - (*cl_p()).pendingcmd.servertime as f64) as c_float;
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1064 -- CL_SendCmd

unsafe fn cl_send_cmd() -> Raise {
    // SAFETY: `cl`/`cls` are initialised; `cmd` is a live local.
    unsafe {
        if (*cls_p()).state != CA_CONNECTED {
            return 0;
        }

        let mut cmd: quake_types::host::UserCmd = core::mem::zeroed();

        crate::cl_input::CL_BaseMove(ptr::addr_of_mut!(cmd).cast());

        cmd.forwardmove +=
            (*cl_p()).pendingcmd.forwardmove + (*cl_p()).pendingcmd.forwardmove_accumulator;
        cmd.sidemove += (*cl_p()).pendingcmd.sidemove + (*cl_p()).pendingcmd.sidemove_accumulator;
        cmd.upmove += (*cl_p()).pendingcmd.upmove + (*cl_p()).pendingcmd.upmove_accumulator;
        cmd.sequence = (*cl_p()).movemessages as c_uint;
        cmd.servertime = (*cl_p()).time as c_float;
        cmd.seconds = cmd.servertime - (*cl_p()).pendingcmd.servertime;

        crate::cl_input::CL_FinishMove(ptr::addr_of_mut!(cmd).cast());

        if (*cls_p()).signon == SIGNONS {
            raise!(crate::cl_input::quake_rs_cl_send_move(
                ptr::addr_of_mut!(cmd).cast()
            ));
        } else {
            raise!(crate::cl_input::quake_rs_cl_send_move(ptr::null_mut()));
        }
        ptr::write_bytes(
            ptr::addr_of_mut!((*cl_p()).pendingcmd).cast::<u8>(),
            0,
            core::mem::size_of::<quake_types::host::UserCmd>(),
        );
        (*cl_p()).pendingcmd.servertime = cmd.servertime;

        let msg = ptr::addr_of_mut!((*cls_p()).message);
        if (*cls_p()).demoplayback {
            c::cvar_cmd::SZ_Clear(msg.cast());
            return 0;
        }

        if (*cls_p()).message.cursize == 0 {
            return 0;
        }

        let mut can: c_int = 0;
        raise!(g::ClMain_Glue_NetCanSendMessage(
            (*cls_p()).netcon.cast(),
            ptr::addr_of_mut!(can)
        ));
        if can == 0 {
            c::Con_DPrintf(c"CL_SendCmd: can't send\n".as_ptr());
            return 0;
        }

        let mut sent: c_int = 0;
        raise!(g::ClMain_Glue_NetSendMessage(
            (*cls_p()).netcon.cast(),
            msg.cast(),
            ptr::addr_of_mut!(sent)
        ));
        if sent == -1 {
            return RAISE_LOST_SEND;
        }

        c::cvar_cmd::SZ_Clear(msg.cast());
    }
    0
}

/// `cl_main.c:1064`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_send_cmd() -> c_int {
    // SAFETY: called from `cl_main_glue.c` with the engine initialised.
    unsafe { cl_send_cmd() }
}

// ---------------------------------------------------------------------------
// cl_main.c:1120 -- CL_Tracepos_f

/// `cl_main.c:1120`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_tracepos_f() -> c_int {
    // SAFETY: `r_refdef`/`vpn` are live C objects.
    unsafe {
        if (*cls_p()).state != CA_CONNECTED {
            return 0;
        }

        let mut v = [0.0f32; 3];
        let vieworg = r_refdef.vieworg;
        let fwd = g::vpn;
        m::vector_ma(&vieworg, 8192.0, &fwd, &mut v);
        let mut w = [0.0f32; 3];
        raise!(g::ClMain_Glue_TraceLine(
            vieworg.as_ptr(),
            v.as_ptr(),
            w.as_mut_ptr()
        ));

        if m::vector_length(&w) == 0.0 {
            c::Con_Printf(c"Tracepos: trace didn't hit anything\n".as_ptr());
        } else {
            c::Con_Printf(
                c"Tracepos: (%i %i %i)\n".as_ptr(),
                w[0] as c_int,
                w[1] as c_int,
                w[2] as c_int,
            );
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1143 -- CL_Viewpos_f

/// `cl_main.c:1143`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_viewpos_f() -> c_int {
    let mut buf = [0 as c_char; 256];
    // SAFETY: `cl.entities` is populated whenever `cls.state == ca_connected`.
    unsafe {
        if (*cls_p()).state != CA_CONNECTED {
            return 0;
        }

        let ve = ent_at((*cl_p()).viewentity);
        c::cl_main::q_snprintf(
            buf.as_mut_ptr(),
            buf.len(),
            c"(%i %i %i) %i %i %i".as_ptr(),
            (*ve).origin[0] as c_int,
            (*ve).origin[1] as c_int,
            (*ve).origin[2] as c_int,
            (*cl_p()).viewangles[PITCH] as c_int,
            (*cl_p()).viewangles[YAW] as c_int,
            (*cl_p()).viewangles[ROLL] as c_int,
        );

        c::Con_SafePrintf(c"Viewpos: %s\n".as_ptr(), buf.as_ptr());

        if c::Cmd_Argc() >= 2 && c::cvar_cmd::q_strcasecmp(c::Cmd_Argv(1), c"copy".as_ptr()) == 0 {
            g::ClMain_Glue_SetClipboardText(buf.as_ptr());
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1178 -- CL_Viewpos_Completion_f (static)

/// `cl_main.c:1178`.
///
/// # Safety
/// `partial` must be a valid NUL-terminated string or NULL.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_viewpos_completion_f(partial: *const c_char) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        if c::Cmd_Argc() != 2 {
            return 0;
        }
        c::cl_main::Con_AddToTabList(c"copy".as_ptr(), partial, ptr::null());
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1185/:1190 -- serverinfo extensions (static)

/// `cl_main.c:1185`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_serverext_full_serverinfo_f() -> c_int {
    // SAFETY: `Cmd_Argv` returns a NUL-terminated string; the copy width is
    // the C original's `sizeof (cl.serverinfo)`, which may read past the
    // argument's terminator exactly as `memcpy` does in C.
    unsafe {
        let newserverinfo = c::Cmd_Argv(1);
        ptr::copy_nonoverlapping(
            newserverinfo,
            (*cl_p()).serverinfo.as_mut_ptr(),
            (*cl_p()).serverinfo.len(),
        );
    }
    0
}

/// `cl_main.c:1190`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_serverext_serverinfo_update_f() -> c_int {
    // SAFETY: `Cmd_Argv` returns NUL-terminated strings.
    unsafe {
        let newserverkey = c::Cmd_Argv(1);
        let newservervalue = c::Cmd_Argv(2);
        c::cvar_cmd::Info_SetKey(
            (*cl_p()).serverinfo.as_mut_ptr(),
            (*cl_p()).serverinfo.len(),
            newserverkey,
            newservervalue,
        );
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1197 -- CL_UserinfoChanged (static)

/// # Safety
/// `sb` must point into `cl.scores`.
unsafe fn cl_userinfo_changed(sb: *mut ScoreBoard) -> Raise {
    let mut tmp = [0 as c_char; 64];
    // SAFETY: the caller's contract.
    unsafe {
        c::cl_main::Info_GetKey(
            (*sb).userinfo.as_ptr(),
            c"name".as_ptr(),
            (*sb).name.as_mut_ptr(),
            (*sb).name.len(),
        );

        c::cl_main::Info_GetKey(
            (*sb).userinfo.as_ptr(),
            c"topcolor".as_ptr(),
            tmp.as_mut_ptr(),
            tmp.len(),
        );
        let mut colors =
            ((c::cl_main::strtoul(tmp.as_ptr(), ptr::null_mut(), 0) & 0xf) as c_int) << 4;
        c::cl_main::Info_GetKey(
            (*sb).userinfo.as_ptr(),
            c"bottomcolor".as_ptr(),
            tmp.as_mut_ptr(),
            tmp.len(),
        );
        colors |= (c::cl_main::strtoul(tmp.as_ptr(), ptr::null_mut(), 0) & 0xf) as c_int;

        if colors != (*sb).colors {
            (*sb).colors = colors;
            let slot = sb.offset_from((*cl_p()).scores) as c_int;
            raise!(g::ClMain_Glue_TranslateNewPlayerSkin(slot));
        }
    }
    0
}

/// `cl_main.c:1214`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_serverext_full_userinfo_f() -> c_int {
    // SAFETY: `Cmd_Argv` returns NUL-terminated strings; the slot is bounds-
    // checked against `cl.maxclients` exactly as the C original does.
    unsafe {
        let slot = c::sv_main::atoi(c::Cmd_Argv(1));
        let newserverinfo = c::Cmd_Argv(2);
        if slot < (*cl_p()).maxclients {
            let sb = score_at(slot);
            c::cl_main::strncpy(
                (*sb).userinfo.as_mut_ptr(),
                newserverinfo,
                (*sb).userinfo.len() - 1,
            );
            raise!(cl_userinfo_changed(sb));
        }
    }
    0
}

/// `cl_main.c:1225`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_serverext_userinfo_update_f() -> c_int {
    // SAFETY: as above.
    unsafe {
        let slot = c::sv_main::atoi(c::Cmd_Argv(1));
        let newserverkey = c::Cmd_Argv(2);
        let newservervalue = c::Cmd_Argv(3);
        if slot < (*cl_p()).maxclients {
            let sb = score_at(slot);
            c::cvar_cmd::Info_SetKey(
                (*sb).userinfo.as_mut_ptr(),
                (*sb).userinfo.len(),
                newserverkey,
                newservervalue,
            );
            raise!(cl_userinfo_changed(sb));
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1238 -- SV_DecodeUserInfo (static)

/// # Safety
/// `client` must point at a live `client_t` with a valid `edict`.
unsafe fn sv_decode_userinfo(client: *mut Client) -> Raise {
    let mut tmp = [0 as c_char; 64];
    // SAFETY: the caller's contract.
    unsafe {
        c::cl_main::Info_GetKey(
            (*client).userinfo.as_ptr(),
            c"topcolor".as_ptr(),
            tmp.as_mut_ptr(),
            tmp.len(),
        );
        let mut top = c::sv_main::atoi(tmp.as_ptr()) & 15;
        if top > 13 {
            top = 13;
        }
        c::cl_main::Info_GetKey(
            (*client).userinfo.as_ptr(),
            c"bottomcolor".as_ptr(),
            tmp.as_mut_ptr(),
            tmp.len(),
        );
        let mut bot = c::sv_main::atoi(tmp.as_ptr()) & 15;
        if bot > 13 {
            bot = 13;
        }
        (*(*client).edict).v.team = (bot + 1) as f32;
        (*client).colors = (top << 4) | bot;

        c::cl_main::Info_GetKey(
            (*client).userinfo.as_ptr(),
            c"name".as_ptr(),
            tmp.as_mut_ptr(),
            tmp.len(),
        );

        if tmp[0] == 0 {
            c::cl_main::q_strlcpy(tmp.as_mut_ptr(), c"unnamed".as_ptr(), tmp.len());
        }

        if c::cl_main::strcmp((*client).name.as_ptr(), tmp.as_ptr()) != 0 {
            if (*client).name[0] != 0
                && c::cl_main::strcmp((*client).name.as_ptr(), c"unconnected".as_ptr()) != 0
            {
                c::Con_DPrintf(
                    c"\"%s\" renamed to \"%s\"\n".as_ptr(),
                    (*client).name.as_ptr(),
                    tmp.as_ptr(),
                );
            }
            c::cl_main::strcpy((*client).name.as_mut_ptr(), tmp.as_ptr());

            let mut s: c_int = 0;
            raise!(g::ClMain_Glue_PRSetEngineString(
                (*client).name.as_ptr(),
                ptr::addr_of_mut!(s)
            ));
            (*(*client).edict).v.netname = s;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1271 -- SV_UpdateInfo

unsafe fn sv_update_info(edict: c_int, keyname: *const c_char, value: *const c_char) -> Raise {
    let mut oldvalue = [0 as c_char; 1024];
    // SAFETY: `svs`/`sv` are initialised; `edict` is range-checked below.
    unsafe {
        let mut edict = edict;
        let mut value = value;
        let info: *mut c_char;
        let infosize: usize;
        let pre: *const c_char;
        let mut infoplayer: *mut Client = ptr::null_mut();

        if edict == 0 {
            let mut name: *const c_char = ptr::null();
            if g::ClMain_Glue_FindServerinfoCvar(keyname, ptr::addr_of_mut!(name)) != 0 {
                raise!(g::ClMain_Glue_CvarSet(name, value));
                return 0;
            }
            let si = ptr::addr_of_mut!(crate::sv_main::svs.serverinfo);
            info = si.cast::<c_char>();
            infosize = core::mem::size_of_val(&*si);
            pre = c"//svi ".as_ptr();
        } else if edict <= crate::sv_main::svs.maxclients {
            edict -= 1;
            infoplayer = crate::sv_main::svs.clients.offset(edict as isize);
            info = (*infoplayer).userinfo.as_mut_ptr();
            infosize = (*infoplayer).userinfo.len();
            pre = c::cl_main::va(c"//ui %i".as_ptr(), edict);
        } else {
            return 0;
        }

        c::cl_main::Info_GetKey(info, keyname, oldvalue.as_mut_ptr(), oldvalue.len());

        if c::cl_main::strcmp(value, oldvalue.as_ptr()) != 0 {
            c::cvar_cmd::Info_SetKey(info, infosize, keyname, value);

            if !infoplayer.is_null() {
                raise!(sv_decode_userinfo(infoplayer));
            }

            if *keyname == b'_' as c_char || !crate::sv_main::sv.active {
                return 0;
            }

            c::cl_main::Info_GetKey(info, keyname, oldvalue.as_mut_ptr(), oldvalue.len());
            value = oldvalue.as_ptr();

            let clients = crate::sv_main::svs.clients;
            for n in 0..crate::sv_main::svs.maxclients {
                let current_client = clients.offset(n as isize);
                if (*current_client).active {
                    let msg = ptr::addr_of_mut!((*current_client).message).cast();
                    let mut w = Writer::new(msg);
                    if (*current_client).protocol_pext2 & PEXT2_PREDINFO != 0 {
                        raise!(w.byte(SVC_STUFFTEXT));
                        raise!(w.string(c::cl_main::va(
                            c"%s \"%s\" \"%s\"\n".as_ptr(),
                            pre,
                            keyname,
                            value
                        )));
                        raise!(w.flush());
                    } else if !infoplayer.is_null()
                        && c::cl_main::strcmp(keyname, c"name".as_ptr()) == 0
                    {
                        raise!(w.byte(SVC_UPDATENAME));
                        raise!(w.byte(edict));
                        raise!(w.string(value));
                        raise!(w.flush());
                    } else if !infoplayer.is_null()
                        && (c::cl_main::strcmp(keyname, c"topcolor".as_ptr()) == 0
                            || c::cl_main::strcmp(keyname, c"bottomcolor".as_ptr()) == 0)
                    {
                        raise!(w.byte(SVC_UPDATECOLORS));
                        raise!(w.byte(edict));
                        raise!(w.byte((*infoplayer).colors));
                        raise!(w.flush());
                    }
                }
            }
        }
    }
    0
}

/// `cl_main.c:1271`.
///
/// # Safety
/// `keyname` and `value` must be valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_update_info(
    edict: c_int,
    keyname: *const c_char,
    value: *const c_char,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { sv_update_info(edict, keyname, value) }
}

// ---------------------------------------------------------------------------
// cl_main.c:1345/:1350 -- the remaining static handlers

/// `cl_main.c:1345`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_serverext_ignore_f() -> c_int {
    // SAFETY: `Cmd_Argv (0)` is always a NUL-terminated string.
    unsafe {
        c::Con_DPrintf2(c"Ignoring stufftext: %s\n".as_ptr(), c::Cmd_Argv(0));
    }
    0
}

/// `cl_main.c:1350`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_legacy_color_f() -> c_int {
    // SAFETY: `Cmd_Argv (1)` is always a NUL-terminated string.
    unsafe {
        let col = c::sv_main::atoi(c::Cmd_Argv(1));
        raise!(g::ClMain_Glue_CvarSetValue(
            c"topcolor".as_ptr(),
            ((col >> 4) & 0xf) as c_float
        ));
        raise!(g::ClMain_Glue_CvarSetValue(
            c"bottomcolor".as_ptr(),
            ((col) & 0xf) as c_float
        ));
    }
    0
}

// ---------------------------------------------------------------------------
// cl_main.c:1363 -- CL_Init

extern "C" {
    /* The plain, re-raising entry points of cl_main_glue.c, registered as
    command handlers so no Rust frame sits under a longjmp. */
    fn CL_LegacyColor_f();
    fn CL_PrintEntities_f();
    fn CL_Disconnect_f();
    fn CL_Tracepos_f();
    fn CL_Viewpos_f();
    fn CL_ServerExtension_FullServerinfo_f();
    fn CL_ServerExtension_ServerinfoUpdate_f();
    fn CL_ServerExtension_FullUserinfo_f();
    fn CL_ServerExtension_UserinfoUpdate_f();
    fn CL_ServerExtension_Ignore_f();
    /* cl_demo_glue.c's entry points. */
    fn CL_Record_f();
    fn CL_Stop_f();
    fn CL_PlayDemo_f();
    fn CL_TimeDemo_f();
    fn CL_Seek_f();
}

/// `cmd.h:110` -- `Cmd_AddCommand (name, func)`.
fn add_command(name: *const c_char, func: unsafe extern "C" fn()) -> *mut c::cmd_function_s {
    // SAFETY: `name` is a static NUL-terminated literal and `func` has the
    // `xcommand_t` signature.
    unsafe { c::Cmd_AddCommand2(name, Some(func), c::cmd_source_t_src_command, false) }
}

/// `cmd.h:112` -- `Cmd_AddCommand_ServerCommand (name, func)`.
fn add_server_command(name: *const c_char, func: unsafe extern "C" fn()) {
    // SAFETY: as `add_command`.
    unsafe {
        c::Cmd_AddCommand2(name, Some(func), c::cmd_source_t_src_server, false);
    }
}

/// `cl_main.c:1363`.
#[no_mangle]
pub extern "C" fn quake_rs_cl_init() -> c_int {
    // SAFETY: every cvar below is a live C object in `cl_main_glue.c`,
    // `cl_input_glue.c` or `chase_glue.c`, and registration order is
    // observable in `config.cfg`, so it is preserved verbatim.
    unsafe {
        c::cvar_cmd::SZ_Alloc(ptr::addr_of_mut!((*cls_p()).message).cast(), 1024);

        crate::cl_input::CL_InitInput();
        crate::cl_tent::CL_InitTEnts();

        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_name
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_topcolor
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_bottomcolor
        )));
        add_command(c"_cl_color".as_ptr(), CL_LegacyColor_f);
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_upspeed
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_forwardspeed
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_backspeed
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_sidespeed
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_movespeedkey
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_yawspeed
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_pitchspeed
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_anglespeedkey
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_shownet
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_nolerp
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::lookspring
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::lookstrafe
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::sensitivity
        )));

        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_alwaysrun
        )));

        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::m_pitch
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(g::m_yaw)));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::m_forward
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::m_side
        )));

        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cfg_unbindall
        )));

        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_maxpitch
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::cl_input::cl_minpitch
        )));

        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_startdemos
        )));
        raise!(g::ClMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::cl_confirmquit
        )));

        add_command(c"entities".as_ptr(), CL_PrintEntities_f);
        add_command(c"disconnect".as_ptr(), CL_Disconnect_f);
        add_command(c"record".as_ptr(), CL_Record_f);
        add_command(c"stop".as_ptr(), CL_Stop_f);
        add_command(c"playdemo".as_ptr(), CL_PlayDemo_f);
        add_command(c"timedemo".as_ptr(), CL_TimeDemo_f);
        add_command(c"seek".as_ptr(), CL_Seek_f);

        add_command(c"tracepos".as_ptr(), CL_Tracepos_f);
        let cmd = add_command(c"viewpos".as_ptr(), CL_Viewpos_f);
        if !cmd.is_null() {
            g::ClMain_Glue_SetViewposCompletion(cmd.cast());
        }

        add_server_command(
            c"fullserverinfo".as_ptr(),
            CL_ServerExtension_FullServerinfo_f,
        );
        add_server_command(c"svi".as_ptr(), CL_ServerExtension_ServerinfoUpdate_f);

        add_server_command(c"fui".as_ptr(), CL_ServerExtension_FullUserinfo_f);
        add_server_command(c"ui".as_ptr(), CL_ServerExtension_UserinfoUpdate_f);

        add_server_command(c"paknames".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"paks".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"wps".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"it".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"tinfo".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"exectrigger".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"csqc_progname".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"csqc_progsize".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"csqc_progcrc".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"cl_fullpitch".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"pq_fullpitch".as_ptr(), CL_ServerExtension_Ignore_f);

        add_server_command(
            c"cl_serverextension_download".as_ptr(),
            CL_ServerExtension_Ignore_f,
        );
        add_server_command(c"cl_downloadbegin".as_ptr(), CL_ServerExtension_Ignore_f);
        add_server_command(c"cl_downloadfinished".as_ptr(), CL_ServerExtension_Ignore_f);
    }
    0
}
