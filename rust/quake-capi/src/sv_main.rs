//! C ABI shims for `Quake/sv_main.c` -- the server connection half (Rust
//! migration Phase 7 M6, T6.5).
//!
//! Near-transliteration of `SV_Init`, the `svc_particle`/`svc_sound`/
//! `svc_localsound` event writers, the client-spawning path
//! (`SV_SendServerinfo`/`SV_Pext_f`/`SV_ConnectClient`/
//! `SV_CheckForNewClients`), the precache lookups and `SV_SpawnServer`.
//! `localmodels` (`sv_main.c:30`) had internal linkage in C, so it is a
//! Rust-owned array here; `sv`, `svs`, `sv_protocol`, `sv_protocol_pext1/2`,
//! `sv_netsort` and `sv_smoothplatformlerps` stay C-visible storage in
//! `Quake/sv_main_glue.c` (`sv`/`svs` move to Rust at T6.6 per ADR-007).
//!
//! ADR-011: `sv`, `svs`, `cl`, `cls` and `client_t` are reached directly
//! through the hand-written mirrors in `quake-types`, never through per-field
//! glue accessors. The four `static mut` externs below are declared here
//! rather than in `quake-c-sys` because that crate has no `[dependencies]`
//! and so cannot name the mirror types (recorded in full in
//! `quake-c-sys/src/sv_main.rs`).
//!
//! ADR-009 audit. Direct raise sites in the C file: `Sys_Error` at `:213`,
//! `:784` and `:835` (terminates the process -- it does not `longjmp` through
//! `Host_Guard`, so those are called straight through), and `Host_Error` at
//! `:285`, `:292` and `:295` (the three `SV_StartSound` argument checks, now
//! `SvMain_Glue_Error*`). The transitive set is: every `MSG_Write*` /
//! `MSG_WriteString` (`SZ_GetSpace` -> `Host_Error` when the sizebuf
//! disallows overflow, `net_msg.c:493`), `PR_GetString`
//! (`pr_edict_arena.c`), `EDICT_NUM` / `NUM_FOR_EDICT`,
//! `PR_ExecuteProgram (SetNewParms)` and `(SetChangeParms)`,
//! `SVFTE_SetupFrames`, `SV_SendReconnect`, `SV_CreateBaseline`,
//! `Host_ClearMemory`, `PR_LoadProgs`, `Mod_ForName`, `ED_LoadFromFile`,
//! `SV_Precache_Model`, `SV_Physics`, and -- under `-Duse_rust_cvar`, where
//! the plain `Cvar_*`/`Cmd_*` names are themselves `Host_Reraise` wrappers --
//! `Cvar_RegisterVariable`, `Cvar_Set`, `Cvar_SetValue` and `Cmd_AddCommand`.
//! Each of those reaches C through a `SvMain_Glue_*` / `World_Glue_*`
//! trampoline that runs it inside one `Host_Guard` frame and returns the
//! status; a non-zero status is returned to the caller immediately, so every
//! function abandons its remaining work exactly where C's `longjmp` would
//! have left it. No jump ever unwinds a Rust frame.
//!
//! Message writes are batched through `SvMain_Glue_WriteBatch`: one guarded
//! C frame executes a run of `MSG_Write*` calls in order. Batching is
//! behaviourally identical to per-call guarding because a raise aborts the
//! whole remaining sequence either way, and it keeps `SV_SendServerinfo`'s
//! precache loops (up to `MAX_MODELS` entries) off a fixed-size stack array.
//!
//! ADR-005: `Quake/sv_main.c` has exactly one floating-point conversion
//! specifier, the `%f` in `Host_Error ("SV_StartSound: attenuation = %f")`
//! (`sv_main.c:293`); it lives in `SvMain_Glue_ErrorAttenuation`, so no Rust
//! formatter is involved. There are no `%g` and no `%e` sites at all.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::sv_main as g;
use quake_c_sys::world as w;
use quake_types::host::{
    Client, ClientState, ClientStatic, Server, ServerStatic, MAX_MODELS, PRESPAWN_DONE,
    PRESPAWN_FLUSH, SS_ACTIVE, SS_LOADING,
};
use quake_types::model_mem::{QModel, MOD_BRUSH};
use quake_types::net::{SizeBuf, MAX_DATAGRAM, MAX_MSGLEN};
use quake_types::progs::{Edict, GlobalVars, QcVm};
use quake_types::sound::MAX_SOUNDS;

/// A `Host_Guard` status: `HOST_GUARD_OK` (0) or the code the guarded frame
/// caught. Non-zero must be returned to `Quake/sv_main_glue.c` untouched.
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

/// The server state, `sv_main.c:27`. T6.6 moved this storage out of C and
/// closed the last ADR-007 sv/svs dual view: `Quake/server.h`'s
/// `extern server_t sv;` is unchanged, and the 35 C translation units that
/// read `sv.` now resolve to this definition. C zero-initialises it in BSS,
/// so the initialiser is a zeroed `Server`; the ADR-011 layout gate is
/// `rust/quake-ctest/tests/host_abi.rs`.
///
/// SAFETY: every field of `Server` is a raw pointer, an integer, a `bool` or
/// an array of those, so all-zero is a valid value for each.
#[no_mangle]
pub static mut sv: Server = unsafe { core::mem::zeroed() };

/// The persistent server state, `sv_main.c:28`. Same ownership story as
/// [`sv`] above.
///
/// SAFETY: as [`sv`].
#[no_mangle]
pub static mut svs: ServerStatic = unsafe { core::mem::zeroed() };

extern "C" {
    /// `Quake/host.c` -- the client slot the current command is attributed
    /// to. `SV_SendServerinfo` and `SV_Pext_f` both read it.
    static mut host_client: *mut Client;
    /// `Quake/cl_main.c` -- read only by `SV_Pext_f`'s client-side branch.
    static mut cl: ClientState;
    /// `Quake/cl_main.c` -- same.
    static mut cls: ClientStatic;
}

// ---------------------------------------------------------------------------
// engine constants (protocol.h / quakedef.h / server.h / progdefs.q1)

/// `protocol.h:35`
const PROTOCOL_NETQUAKE: c_int = 15;
/// `protocol.h:36`
const PROTOCOL_FITZQUAKE: c_int = 666;
/// `protocol.h:37`
const PROTOCOL_RMQ: c_int = 999;
/// `protocol.h:35`, as compared against `sv.protocol` (`unsigned int`).
const PROTOCOL_NETQUAKE_U: c_uint = 15;
/// `protocol.h:36`
const PROTOCOL_FITZQUAKE_U: c_uint = 666;
/// `protocol.h:37`
const PROTOCOL_RMQ_U: c_uint = 999;
/// `protocol.h:40` -- `('F'<<0)+('T'<<8)+('E'<<16)+('2'<<24)`.
const PROTOCOL_FTE_PEXT2: c_uint = 0x3245_5446;
/// The same value as the `int` `SV_Pext_f` compares its parsed key against.
const PROTOCOL_FTE_PEXT2_I: c_int = 0x3245_5446;

/// `protocol.h:44`
const PRFL_SHORTANGLE: c_uint = 1 << 1;
/// `protocol.h:47`
const PRFL_FLOATCOORD: c_uint = 1 << 4;
/// `protocol.h:50`
const PRFL_INT32COORD: c_uint = 1 << 7;

/// `protocol.h:55` -- `PEXT1_CSQC`.
const PEXT1_SUPPORTED_SERVER: c_int = 0x4000_0000;
/// `protocol.h:60`
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
/// `protocol.h:61`
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
/// `protocol.h:63`
const PEXT2_SUPPORTED_SERVER: c_int = 0x0000_0028;
/// `protocol.h:63`, as an `unsigned int` mask.
const PEXT2_SUPPORTED_SERVER_U: c_uint = 0x0000_0028;

/// `protocol.h:191`
const SND_VOLUME: c_int = 1 << 0;
/// `protocol.h:192`
const SND_ATTENUATION: c_int = 1 << 1;
/// `protocol.h:199`
const SND_LARGEENTITY: c_int = 1 << 3;
/// `protocol.h:200`
const SND_LARGESOUND: c_int = 1 << 4;
/// `protocol.h:195`
const DEFAULT_SOUND_PACKET_VOLUME: c_int = 255;
/// `protocol.h:196`
const DEFAULT_SOUND_PACKET_ATTENUATION: c_float = 1.0;

/// `protocol.h:250`
const SVC_SETVIEW: c_int = 5;
/// `protocol.h:251`
const SVC_SOUND: c_int = 6;
/// `protocol.h:253`
const SVC_PRINT: c_int = 8;
/// `protocol.h:254`
const SVC_STUFFTEXT: c_int = 9;
/// `protocol.h:258`
const SVC_SERVERINFO: c_int = 11;
/// `protocol.h:269`
const SVC_PARTICLE: c_int = 18;
/// `protocol.h:277`
const SVC_SIGNONNUM: c_int = 25;
/// `protocol.h:284`
const SVC_CDTRACK: c_int = 32;
/// `protocol.h:312`
const SVC_LOCALSOUND: c_int = 56;

/// `quakedef.h` -- `GAME_COOP`.
const GAME_COOP: c_int = 0;
/// `quakedef.h` -- `GAME_DEATHMATCH`.
const GAME_DEATHMATCH: c_int = 1;

/// `net.h:36`
const NET_MAXMESSAGE: c_uint = 64000;
/// `net.h` -- the safe unreliable size for a remote client.
const DATAGRAM_MTU: c_uint = 1400;
/// `quakedef.h`
const MIN_EDICTS: c_int = 256;
/// `quakedef.h`
const MAX_EDICTS: c_int = 32000;
/// `server.h:116`
const NUM_BASIC_SPAWN_PARMS: usize = 16;
/// `server.h:117`
const NUM_TOTAL_SPAWN_PARMS: usize = 64;

/// `progdefs.q1` `SOLID_BSP`.
const SOLID_BSP: c_float = 4.0;
/// `progdefs.q1` `MOVETYPE_PUSH`.
const MOVETYPE_PUSH: c_float = 7.0;

/// `cmd.h` `src_client`.
const SRC_CLIENT: c::cmd_source_t = c::cmd_source_t_src_client;

// ---------------------------------------------------------------------------
// Rust-owned storage

/// `sv_main.c:30` -- `static char localmodels[MAX_MODELS][8]`. Internal
/// linkage in C, so it becomes Rust-owned under the Pattern A swap.
static mut LOCALMODELS: [[c_char; 8]; MAX_MODELS] = [[0; 8]; MAX_MODELS];

/// `sv_main.c:888` -- `static char dummy[8] = {0, ...}`, a function-scope
/// static whose address is stored into `sv.sound_precache[0]` and
/// `sv.model_precache[0]`, so it must outlive the call.
static mut DUMMY: [c_char; 8] = [0; 8];

// ---------------------------------------------------------------------------
// small helpers

/// `&sv` without forming a reference to the `static mut`.
#[inline]
fn sv_p() -> *mut Server {
    ptr::addr_of_mut!(sv)
}

/// `&svs` without forming a reference to the `static mut`.
#[inline]
fn svs_p() -> *mut ServerStatic {
    ptr::addr_of_mut!(svs)
}

/// The current value of the C global `host_client`.
#[inline]
unsafe fn host_client_get() -> *mut Client {
    // SAFETY: single-threaded engine state; the read cannot trap.
    unsafe { ptr::addr_of_mut!(host_client).read() }
}

/// Stores into the C global `host_client`.
#[inline]
unsafe fn host_client_set(v: *mut Client) {
    // SAFETY: single-threaded engine state.
    unsafe { ptr::addr_of_mut!(host_client).write(v) }
}

/// Reads a C `cvar_t`'s `.value` without forming a reference to the static.
#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: `var` always points at a `cvar_t` static owned by the engine;
    // cvars are single-threaded state.
    unsafe { ptr::addr_of!((*var).value).read() }
}

/// The ambient qcvm (ADR-008).
#[inline]
unsafe fn vm() -> *mut QcVm {
    // SAFETY: single-threaded engine state.
    unsafe { ptr::addr_of_mut!(c::qcvm).read().cast::<QcVm>() }
}

/// `net_msg.c:475` `SZ_Clear` -- two field stores, no raise, so it is
/// reimplemented here instead of crossing the FFI boundary.
#[inline]
unsafe fn sz_clear(buf: *mut SizeBuf) {
    // SAFETY: `buf` points at a live `sizebuf_t` owned by `sv` or a client.
    unsafe {
        (*buf).cursize = 0;
        (*buf).overflowed = false;
    }
}

/// `common.c` `q_strlcpy` -- truncating bounded copy, always NUL-terminated.
/// Cannot raise, so it stays in Rust.
unsafe fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) {
    // SAFETY: callers pass a fixed-size destination array and its length.
    unsafe {
        if size == 0 {
            return;
        }
        let mut i = 0usize;
        while i + 1 < size {
            let ch = *src.add(i);
            if ch == 0 {
                break;
            }
            *dst.add(i) = ch;
            i += 1;
        }
        *dst.add(i) = 0;
    }
}

/// `q_snprintf (dst, size, "maps/%s.bsp", server)` (`sv_main.c:995`).
/// `q_vsnprintf` (`common.c:617`) truncates and NUL-terminates and never
/// raises, so the conversion is reproduced directly.
unsafe fn format_map_name(dst: *mut c_char, size: usize, server: *const c_char) {
    // SAFETY: `dst` is `sv.modelname`, a `char[64]`; `server` is NUL-terminated.
    unsafe {
        if size == 0 {
            return;
        }
        let mut n = 0usize;
        let put = |b: c_char, n: &mut usize| {
            if *n + 1 < size {
                *dst.add(*n) = b;
            }
            *n += 1;
        };
        for &b in b"maps/" {
            put(b as c_char, &mut n);
        }
        let mut k = 0usize;
        loop {
            let ch = *server.add(k);
            if ch == 0 {
                break;
            }
            put(ch, &mut n);
            k += 1;
        }
        for &b in b".bsp" {
            put(b as c_char, &mut n);
        }
        let end = if n < size { n } else { size - 1 };
        *dst.add(end) = 0;
    }
}

/// `q_snprintf (localmodels[i], 8, "*%i", i)` (`sv_main.c:196`).
unsafe fn format_local_model(dst: *mut c_char, size: usize, i: c_int) {
    // SAFETY: `dst` is one `char[8]` row of `LOCALMODELS`.
    unsafe {
        if size == 0 {
            return;
        }
        let mut digits = [0u8; 12];
        let mut nd = 0usize;
        let mut v = i as u32;
        if v == 0 {
            digits[0] = b'0';
            nd = 1;
        } else {
            while v > 0 {
                digits[nd] = b'0' + (v % 10) as u8;
                v /= 10;
                nd += 1;
            }
        }
        let mut n = 0usize;
        if n + 1 < size {
            *dst.add(n) = b'*' as c_char;
        }
        n += 1;
        for k in (0..nd).rev() {
            if n + 1 < size {
                *dst.add(n) = digits[k] as c_char;
            }
            n += 1;
        }
        let end = if n < size { n } else { size - 1 };
        *dst.add(end) = 0;
    }
}

// ---------------------------------------------------------------------------
// guarded message writing

/// How many `MSG_Write*` operations are buffered before one guarded C frame
/// executes them. Purely a stack-size choice: a raise aborts the whole
/// remaining sequence at the same op either way, so the observable byte
/// stream is identical for any capacity.
const WRITE_BATCH: usize = 64;

/// `svmain_write_t.kind` values -- must match `Quake/sv_main_glue.c`.
const W_BYTE: c_int = 0;
const W_CHAR: c_int = 1;
const W_SHORT: c_int = 2;
const W_LONG: c_int = 3;
const W_STRING: c_int = 4;
const W_COORD: c_int = 5;

/// Accumulates `MSG_Write*` calls against one `sizebuf_t` and hands them to
/// `SvMain_Glue_WriteBatch`, which runs the whole run inside a single
/// `Host_Guard` frame (ADR-009 rule 3).
struct Writer {
    sb: *mut c_void,
    ops: [g::SvMainWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new(sb: *mut c_void) -> Self {
        Writer {
            sb,
            ops: [g::SvMainWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                s: ptr::null(),
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
        // initialised and every `s` pointer is still live at this point.
        unsafe { g::SvMain_Glue_WriteBatch(self.sb, self.ops.as_ptr(), count as c_int) }
    }

    unsafe fn push(&mut self, kind: c_int, i: c_int, f: c_float, s: *const c_char) -> Raise {
        if self.n == WRITE_BATCH {
            // SAFETY: see `flush`.
            let r = unsafe { self.flush() };
            if r != 0 {
                return r;
            }
        }
        self.ops[self.n] = g::SvMainWriteOp { kind, i, f, s };
        self.n += 1;
        0
    }

    unsafe fn byte(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_BYTE, v, 0.0, ptr::null()) }
    }

    unsafe fn char_(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_CHAR, v, 0.0, ptr::null()) }
    }

    unsafe fn short(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_SHORT, v, 0.0, ptr::null()) }
    }

    unsafe fn long(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_LONG, v, 0.0, ptr::null()) }
    }

    unsafe fn string(&mut self, s: *const c_char) -> Raise {
        // SAFETY: see `push`; `s` must stay live until the next flush.
        unsafe { self.push(W_STRING, 0, 0.0, s) }
    }

    /// `MSG_WriteCoord (sb, f, sv.protocolflags)` -- the flags argument is
    /// read inside the glue, matching `Quake/pr_cmds_glue.c:295`.
    unsafe fn coord(&mut self, f: c_float) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_COORD, 0, f, ptr::null()) }
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:49 SV_Protocol_f

/// `sv_main.c:49` `SV_Protocol_f`. `static` in C; reached from the engine
/// only through the `sv_protocol` command `Quake/sv_main_glue.c` registers.
/// Nothing in the body can raise (`Con_Printf`/`Con_SafePrintf` only), so it
/// is not statusized.
///
/// # Safety
/// Must be called on the main thread with a tokenized command line.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_protocol_f() {
    // SAFETY: single-threaded engine state; `Cmd_Argv` returns a live
    // NUL-terminated token for any index the tokenizer produced.
    unsafe {
        let prot = ptr::addr_of_mut!(g::sv_protocol).read();
        let mut pext1 = ptr::addr_of_mut!(g::sv_protocol_pext1).read() as c_int;
        let mut pext2 = ptr::addr_of_mut!(g::sv_protocol_pext2).read() as c_int;

        match c::Cmd_Argc() {
            1 => {
                let fte = if ptr::addr_of_mut!(g::sv_protocol_pext2).read() != 0 {
                    c"fte".as_ptr()
                } else {
                    c"".as_ptr()
                };
                c::Con_Printf(c"\"sv_protocol\" is \"%s%i\"\n".as_ptr(), fte, prot);
            }
            2 => {
                let mut s: *const c_char = c::Cmd_Argv(1);
                if g::q_strncasecmp(s, c"FTE".as_ptr(), 3) == 0 {
                    s = s.add(3);
                    if *s == b'+' as c_char || *s == b'-' as c_char {
                        s = s.add(1);
                    }
                    pext1 = PEXT1_SUPPORTED_SERVER;
                    pext2 = PEXT2_SUPPORTED_SERVER;
                }
                // COMPAT: sv_main.c:75 passes n = 3 for the one-character
                // needle "+"; q_strncasecmp stops at the needle's NUL, so the
                // extra length is harmless but is preserved verbatim.
                else if g::q_strncasecmp(s, c"+".as_ptr(), 3) == 0 {
                    s = s.add(1);
                    pext1 = PEXT1_SUPPORTED_SERVER;
                    pext2 = PEXT2_SUPPORTED_SERVER;
                } else if g::q_strncasecmp(s, c"Base".as_ptr(), 4) == 0 {
                    s = s.add(4);
                    if *s == b'+' as c_char || *s == b'-' as c_char {
                        s = s.add(1);
                    }
                    pext1 = 0;
                    pext2 = 0;
                } else if *s == b'-' as c_char {
                    s = s.add(1);
                    pext1 = 0;
                    pext2 = 0;
                }

                let mut end: *mut c_char = ptr::null_mut();
                let i = g::strtol(s, &mut end, 0) as c_int;
                s = end;
                if *s == b'-' as c_char {
                    pext1 = 0;
                    pext2 = 0;
                } else if *s == b'+' as c_char {
                    pext1 = PEXT1_SUPPORTED_SERVER;
                    pext2 = PEXT2_SUPPORTED_SERVER;
                }

                if i != PROTOCOL_NETQUAKE && i != PROTOCOL_FITZQUAKE && i != PROTOCOL_RMQ {
                    c::Con_Printf(
                        c"sv_protocol must be %i or %i or %i.\nProtocol may be prefixed with FTE+ or Base- to enable/disable FTE extensions.\n".as_ptr(),
                        PROTOCOL_NETQUAKE,
                        PROTOCOL_FITZQUAKE,
                        PROTOCOL_RMQ,
                    );
                } else {
                    ptr::addr_of_mut!(g::sv_protocol).write(i);
                    ptr::addr_of_mut!(g::sv_protocol_pext1).write(pext1 as c_uint);
                    ptr::addr_of_mut!(g::sv_protocol_pext2).write(pext2 as c_uint);
                    if (*sv_p()).active {
                        if prot == ptr::addr_of_mut!(g::sv_protocol).read()
                            && pext1 as c_uint == ptr::addr_of_mut!(g::sv_protocol_pext1).read()
                            && pext2 as c_uint == ptr::addr_of_mut!(g::sv_protocol_pext2).read()
                        {
                            c::Con_Printf(c"specified protocol already active.\n".as_ptr());
                        } else {
                            c::Con_Printf(
                                c"changes will not take effect until the next level load.\n"
                                    .as_ptr(),
                            );
                        }
                    }
                }
            }
            _ => {
                c::Con_SafePrintf(c"usage: sv_protocol <protocol>\n".as_ptr());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:137 SV_Init

/// `sv_main.c:137` `SV_Init`.
///
/// The 23 `Cvar_RegisterVariable` calls keep their exact C order: under
/// `-Duse_rust_cvar` registration order is observable through
/// `Cvar_SetQuick`'s `CVAR_SERVERINFO` replication (`cvar.c:507`), which
/// appends to `svs.serverinfo` in registration order.
///
/// # Safety
/// Must be called once, on the main thread, before any server starts.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_init() -> Raise {
    // SAFETY: single-threaded engine startup; every cvar pointer below names
    // a `cvar_t` static with static storage duration.
    unsafe {
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_maxvelocity
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_gravity
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_friction
        )));
        g::SvMain_Glue_SetNotifyCallback(ptr::addr_of_mut!(c::sv_phys::sv_gravity));
        g::SvMain_Glue_SetNotifyCallback(ptr::addr_of_mut!(c::sv_phys::sv_friction));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_user::sv_edgefriction
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_stopspeed
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_user::sv_maxspeed
        )));
        g::SvMain_Glue_SetNotifyCallback(ptr::addr_of_mut!(c::sv_user::sv_maxspeed));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_user::sv_accelerate
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_user::sv_idealpitchscale
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::progs_builtins_sv::sv_aim
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_nostep
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_freezenonclients
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_gameplayfix_spawnbeforethinks
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_gameplayfix_bouncedownslopes
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_gameplayfix_elevators
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_fastpushmove
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_pushgrid
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_phys::sv_analyticphysics
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            w::pr_checkextension
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            c::sv_user::sv_altnoclip
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::sv_netsort
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            g::sv_smoothplatformlerps
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            w::sv_fte_recursivehullckeck
        )));
        raise!(g::SvMain_Glue_RegisterVariable(ptr::addr_of_mut!(
            w::sv_fte_createareanode
        )));

        raise!(g::SvMain_Glue_AddCommands());

        let base = ptr::addr_of_mut!(LOCALMODELS).cast::<[c_char; 8]>();
        for i in 0..MAX_MODELS {
            format_local_model((*base.add(i)).as_mut_ptr(), 8, i as c_int);
        }

        let mut i = c::COM_CheckParm(c"-protocol".as_ptr());
        if i != 0 && i < c::com_argc - 1 {
            let argv = ptr::addr_of_mut!(c::com_argv).read();
            ptr::addr_of_mut!(g::sv_protocol).write(g::atoi(*argv.add((i + 1) as usize)));
        }
        i = ptr::addr_of_mut!(g::sv_protocol).read();
        let p: *const c_char = match i {
            PROTOCOL_NETQUAKE => c"NetQuake".as_ptr(),
            PROTOCOL_FITZQUAKE => c"FitzQuake".as_ptr(),
            PROTOCOL_RMQ => c"RMQ".as_ptr(),
            _ => {
                // Sys_Error terminates the process; it does not longjmp
                // through Host_Guard, so it is called straight through.
                c::Sys_Error(
                    c"Bad protocol version request %i. Accepted values: %i, %i, %i.".as_ptr(),
                    i,
                    PROTOCOL_NETQUAKE,
                    PROTOCOL_FITZQUAKE,
                    PROTOCOL_RMQ,
                );
            }
        };
        let pext2 = ptr::addr_of_mut!(g::sv_protocol_pext2).read();
        c::Sys_Printf(
            c"Server using protocol %i%s (%s%s)\n".as_ptr(),
            i,
            if pext2 != 0 {
                c"+".as_ptr()
            } else {
                c"".as_ptr()
            },
            if pext2 != 0 {
                c"FTE-".as_ptr()
            } else {
                c"".as_ptr()
            },
            p,
        );
        0
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:234 SV_StartParticle

/// `sv_main.c:234` `SV_StartParticle`.
///
/// # Safety
/// `org` and `dir` must each point at three floats.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_start_particle(
    org: *mut c_float,
    dir: *mut c_float,
    color: c_int,
    count: c_int,
) -> Raise {
    // SAFETY: single-threaded engine state; both vectors are `vec3_t`.
    unsafe {
        let mut count = count;
        if (*sv_p()).datagram.cursize > (*sv_p()).datagram.maxsize - 18 {
            return 0;
        }
        let mut wr = Writer::new(ptr::addr_of_mut!((*sv_p()).datagram).cast());
        raise!(wr.byte(SVC_PARTICLE));
        raise!(wr.coord(*org.add(0)));
        raise!(wr.coord(*org.add(1)));
        raise!(wr.coord(*org.add(2)));
        for i in 0..3usize {
            // C truncates the float toward zero and is undefined out of
            // range; Rust's `as` saturates (rule 8). Both clamp below.
            let v = ((*dir.add(i) * 16.0) as c_int).clamp(-128, 127);
            raise!(wr.char_(v));
        }
        // COMPAT: sv_main.c:253 compares the `int` count against the float
        // literal 255.0f and assigns a float back into an int.
        if (count as c_float) > 255.0 {
            count = 255.0f32 as c_int;
        }
        raise!(wr.byte(count));
        raise!(wr.byte(color));
        wr.flush()
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:277 SV_StartSound

/// `sv_main.c:277` `SV_StartSound`.
///
/// # Safety
/// `entity` must be a live `edict_t *`; `origin`, when non-null, three floats.
// The attenuation range test is spelled as C spells it: `RangeInclusive::
// contains` would also reject NaN, which `attenuation < 0 || attenuation > 4`
// does not (ADR-010).
#[allow(clippy::manual_range_contains)]
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_start_sound(
    entity: *mut c_void,
    origin: *mut c_float,
    channel: c_int,
    sample: *const c_char,
    volume: c_int,
    attenuation: c_float,
) -> Raise {
    // SAFETY: single-threaded engine state; `entity` is a live edict.
    unsafe {
        let entity = entity.cast::<Edict>();
        let mut volume = volume;

        if volume < 0 {
            return g::SvMain_Glue_ErrorVolume(volume);
        } else if volume > 255 {
            volume = 255;
            c::Con_Printf(c"SV_StartSound: volume = %i\n".as_ptr(), volume);
        }

        if attenuation < 0.0 || attenuation > 4.0 {
            // COMPAT: ADR-005 -- the only %f in sv_main.c (:293) stays in C.
            return g::SvMain_Glue_ErrorAttenuation(attenuation);
        }

        if !(0..=255).contains(&channel) {
            return g::SvMain_Glue_ErrorChannel(channel);
        } else if channel > 7 {
            c::Con_DPrintf(c"SV_StartSound: channel = %i\n".as_ptr(), channel);
        }

        let mut sound_num: c_uint = 1;
        while (sound_num as usize) < MAX_SOUNDS
            && !(*sv_p()).sound_precache[sound_num as usize].is_null()
        {
            if g::strcmp(sample, (*sv_p()).sound_precache[sound_num as usize]) == 0 {
                break;
            }
            sound_num += 1;
        }

        if sound_num as usize == MAX_SOUNDS
            || (*sv_p()).sound_precache[sound_num as usize].is_null()
        {
            // COMPAT: sv_main.c:309 misspells "precached" as "precacheed";
            // SV_LocalSound (:399) spells it correctly. Verified: this path
            // returns rather than raising.
            c::Con_Printf(c"SV_StartSound: %s not precacheed\n".as_ptr(), sample);
            return 0;
        }

        let mut ent_num: c_int = 0;
        raise!(w::World_Glue_NumForEdict(entity.cast(), &mut ent_num));
        let ent = ent_num as c_uint;

        let mut field_mask: c_int = 0;
        if volume != DEFAULT_SOUND_PACKET_VOLUME {
            field_mask |= SND_VOLUME;
        }
        if attenuation != DEFAULT_SOUND_PACKET_ATTENUATION {
            field_mask |= SND_ATTENUATION;
        }

        if ent >= 8192 || channel >= 8 {
            field_mask |= SND_LARGEENTITY;
        }
        if sound_num >= 256 {
            field_mask |= SND_LARGESOUND;
        }

        for p in 0..(*svs_p()).maxclients {
            let client = (*svs_p()).clients.add(p as usize);
            if !(*client).active || !(*client).spawned {
                continue;
            }
            if ent >= (*client).limit_entities {
                continue;
            }
            if sound_num >= (*client).limit_sounds {
                continue;
            }
            if (field_mask & (SND_LARGEENTITY | SND_LARGESOUND)) != 0
                && (*sv_p()).protocol == PROTOCOL_NETQUAKE_U
            {
                continue;
            }
            if (*client).datagram.cursize > (*client).datagram.maxsize - 22 {
                continue;
            }

            let mut wr = Writer::new(ptr::addr_of_mut!((*client).datagram).cast());
            raise!(wr.byte(SVC_SOUND));
            raise!(wr.byte(field_mask));
            if field_mask & SND_VOLUME != 0 {
                raise!(wr.byte(volume));
            }
            if field_mask & SND_ATTENUATION != 0 {
                // COMPAT: sv_main.c:349 -- attenuation is validated to <= 4,
                // so `attenuation * 64` can reach 256, one past a byte. The
                // truncation and the (debug-only) range check in
                // MSG_WriteByte are preserved exactly.
                raise!(wr.byte((attenuation * 64.0) as c_int));
            }

            if field_mask & SND_LARGEENTITY != 0 {
                if ((*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS) != 0 && ent > 0x7fff {
                    raise!(wr.short(((ent >> 8) | 0x8000) as c_int));
                    raise!(wr.byte((ent & 0xff) as c_int));
                } else {
                    raise!(wr.short(ent as c_int));
                }
                raise!(wr.byte(channel));
            } else {
                raise!(wr.short(((ent << 3) | channel as c_uint) as c_int));
            }
            if field_mask & SND_LARGESOUND != 0 {
                raise!(wr.short(sound_num as c_int));
            } else {
                raise!(wr.byte(sound_num as c_int));
            }

            for i in 0..3usize {
                if !origin.is_null() {
                    raise!(wr.coord(*origin.add(i)));
                } else {
                    // C promotes the whole expression to double (the 0.5
                    // literal) and narrows once at the call (ADR-010).
                    raise!(wr.coord(
                        ((*entity).v.origin[i] as f64
                            + 0.5 * ((*entity).v.mins[i] as f64 + (*entity).v.maxs[i] as f64))
                            as c_float
                    ));
                }
            }
            raise!(wr.flush());
        }
        0
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:388 SV_LocalSound

/// `sv_main.c:388` `SV_LocalSound` (2021 rerelease).
///
/// # Safety
/// `client` must be a live `client_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_local_sound(
    client: *mut c_void,
    sample: *const c_char,
) -> Raise {
    // SAFETY: single-threaded engine state; `client` is a live slot.
    unsafe {
        let client = client.cast::<Client>();

        let mut sound_num: c_int = 1;
        while (sound_num as usize) < MAX_SOUNDS
            && !(*sv_p()).sound_precache[sound_num as usize].is_null()
        {
            if g::strcmp(sample, (*sv_p()).sound_precache[sound_num as usize]) == 0 {
                break;
            }
            sound_num += 1;
        }
        if sound_num as usize == MAX_SOUNDS
            || (*sv_p()).sound_precache[sound_num as usize].is_null()
        {
            c::Con_Printf(c"SV_LocalSound: %s not precached\n".as_ptr(), sample);
            return 0;
        }

        let mut field_mask: c_int = 0;
        if sound_num >= 256 {
            if (*sv_p()).protocol == PROTOCOL_NETQUAKE_U {
                return 0;
            }
            field_mask = SND_LARGESOUND;
        }

        let mut wr = Writer::new(ptr::addr_of_mut!((*client).message).cast());
        raise!(wr.byte(SVC_LOCALSOUND));
        raise!(wr.byte(field_mask));
        if field_mask & SND_LARGESOUND != 0 {
            raise!(wr.short(sound_num));
        } else {
            raise!(wr.byte(sound_num));
        }
        wr.flush()
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:435 SV_SendServerinfo

/// `sv_main.c:435` `SV_SendServerinfo`.
///
/// # Safety
/// `client_p` must be a live `client_t *`; the ambient qcvm must be the
/// server's (ADR-008).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_send_serverinfo(client_p: *mut c_void) -> Raise {
    // SAFETY: single-threaded engine state; `client_p` is a live slot and
    // every pointer handed to the writer outlives its flush.
    unsafe {
        let client = client_p.cast::<Client>();
        let mut message = [0 as c_char; 2048];
        let mut truncated = false;

        (*client).spawned = false;

        (*client).limit_unreliable = 1024;
        (*client).limit_reliable = 8192;
        (*client).limit_entities = 0;
        (*client).limit_models = 0;
        (*client).limit_sounds = 0;

        let sv_pext2 = ptr::addr_of_mut!(g::sv_protocol_pext2).read();
        if sv_pext2 == 0 {
            (*client).pextknown = false;
        } else if !(*client).pextknown {
            let mut wr = Writer::new(ptr::addr_of_mut!((*client).message).cast());
            raise!(wr.byte(SVC_STUFFTEXT));
            raise!(wr.string(c"cmd pext\n".as_ptr()));
            raise!(wr.flush());
            (*client).sendsignon = PRESPAWN_FLUSH;
            return 0;
        }
        (*client).protocol_pext2 &= sv_pext2;

        if ((*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS) == 0 {
            (*client).protocol_pext2 &= !PEXT2_PREDINFO;
        }

        let selector = if (*client).protocol_pext2 != 0 {
            PROTOCOL_FTE_PEXT2
        } else {
            (*sv_p()).protocol
        };
        match selector {
            PROTOCOL_FITZQUAKE_U => {
                (*client).limit_unreliable = 32000;
                (*client).limit_reliable = 32000;
                (*client).limit_entities = 32000;
                (*client).limit_models = 2048;
                (*client).limit_sounds = 2048;
            }
            PROTOCOL_RMQ_U => {
                (*client).limit_unreliable = 32000;
                (*client).limit_reliable = 64000;
                (*client).limit_entities = 32000;
                (*client).limit_models = 2048;
                (*client).limit_sounds = 2048;
            }
            PROTOCOL_FTE_PEXT2 => {
                (*client).limit_unreliable = NET_MAXMESSAGE;
                (*client).limit_reliable = NET_MAXMESSAGE;
                (*client).limit_entities = MAX_EDICTS as c_uint;
                (*client).limit_models = MAX_MODELS as c_uint;
                (*client).limit_sounds = MAX_SOUNDS as c_uint;
            }
            // `default:` falls into `case PROTOCOL_NETQUAKE:` in C.
            _ => {
                (*client).limit_unreliable = 1024;
                (*client).limit_reliable = 8192;
                if sv_pext2 != 0
                    && g::NET_QSocketGetProQuakeAngleHack((*client).netconnection.cast())
                {
                    (*client).limit_entities = 2048;
                } else {
                    (*client).limit_entities = 600;
                }
                (*client).limit_models = 256;
                (*client).limit_sounds = 256;
            }
        }

        if g::strcmp(
            g::NET_QSocketGetTrueAddressString((*client).netconnection.cast()),
            c"LOCAL".as_ptr(),
        ) == 0
        {
            (*client).limit_unreliable = (*client).limit_reliable;
        } else if (*client).limit_unreliable > DATAGRAM_MTU {
            (*client).limit_unreliable = DATAGRAM_MTU;
        }
        if (*client).limit_entities > 0x8000
            && ((*client).protocol_pext2 & PEXT2_REPLACEMENTDELTAS) == 0
        {
            (*client).limit_entities = 0x8000;
        }
        if (*client).limit_entities > (*vm()).max_edicts as c_uint {
            (*client).limit_entities = (*vm()).max_edicts as c_uint;
        }

        (*client).message.maxsize = MAX_MSGLEN as c_int;
        if (*client).message.maxsize > (*client).limit_reliable as c_int {
            (*client).message.maxsize = (*client).limit_reliable as c_int;
        }
        if (*client).datagram.maxsize > (*client).limit_unreliable as c_int {
            (*client).datagram.maxsize = (*client).limit_unreliable as c_int;
        }

        g::NET_QSocketSetMSS(
            (*client).netconnection.cast(),
            (*client).limit_unreliable as c_int,
        );

        if (*client).message.cursize != 0 {
            // COMPAT: sv_main.c:534-540 flushes `host_client`, not `client`.
            let hc = host_client_get();
            if g::NET_CanSendMessage((*hc).netconnection.cast())
                && g::NET_SendMessage(
                    (*hc).netconnection.cast(),
                    ptr::addr_of_mut!((*hc).message).cast(),
                ) != -1
            {
                sz_clear(ptr::addr_of_mut!((*hc).message));
                (*hc).last_message = ptr::addr_of_mut!(g::realtime).read();
            }
        }

        let cantruncate = (*client).message.cursize == 0;

        // C's `retry:` label plus its `goto retry`.
        loop {
            let mut wr = Writer::new(ptr::addr_of_mut!((*client).message).cast());
            raise!(wr.byte(SVC_PRINT));
            g::SvMain_Glue_ServerinfoPrint(
                message.as_mut_ptr(),
                message.len(),
                (*vm()).progscrc as c_int,
            );
            raise!(wr.string(message.as_ptr()));

            raise!(wr.byte(SVC_SERVERINFO));
            if (*client).protocol_pext2 != 0 {
                raise!(wr.long(PROTOCOL_FTE_PEXT2 as c_int));
                raise!(wr.long((*client).protocol_pext2 as c_int));
            }
            raise!(wr.long((*sv_p()).protocol as c_int));

            if (*sv_p()).protocol == PROTOCOL_RMQ_U {
                raise!(wr.long((*sv_p()).protocolflags as c_int));
            }

            if ((*client).protocol_pext2 & PEXT2_PREDINFO) != 0 {
                raise!(wr.string(g::COM_GetGameNames(false)));
            }

            raise!(wr.byte((*svs_p()).maxclients));

            if cvar_value(ptr::addr_of!(g::coop)) == 0.0
                && cvar_value(ptr::addr_of!(g::deathmatch)) != 0.0
            {
                raise!(wr.byte(GAME_DEATHMATCH));
            } else {
                raise!(wr.byte(GAME_COOP));
            }

            let mut world_message: *const c_char = ptr::null();
            raise!(g::SvMain_Glue_GetString(
                (*(*vm()).edicts).v.message,
                &mut world_message
            ));
            raise!(wr.string(world_message));

            // C walks `sv.model_precache + 1` in lockstep with `i`, so the
            // raw-pointer walk is kept (it can read one past the array when
            // limit_models == MAX_MODELS, exactly as in C).
            let mut i: c_uint = 1;
            let mut s = ptr::addr_of_mut!((*sv_p()).model_precache)
                .cast::<*const c_char>()
                .add(1);
            while !(*s).is_null() && i < (*client).limit_models {
                raise!(wr.string(*s));
                s = s.add(1);
                i += 1;
            }
            raise!(wr.byte(0));
            (*client).signon_models = i;

            // COMPAT: sv_main.c:591 tests `host_client`, not `client`.
            if (*host_client_get()).protocol_pext2 != 0 && truncated {
                i = 1;
            } else {
                i = 1;
                let mut s = ptr::addr_of_mut!((*sv_p()).sound_precache)
                    .cast::<*const c_char>()
                    .add(1);
                while !(*s).is_null() && i < (*client).limit_sounds {
                    raise!(wr.string(*s));
                    s = s.add(1);
                    i += 1;
                }
            }
            raise!(wr.byte(0));
            (*client).signon_sounds = i;

            raise!(wr.byte(SVC_CDTRACK));
            raise!(wr.byte((*(*vm()).edicts).v.sounds as c_int));
            raise!(wr.byte((*(*vm()).edicts).v.sounds as c_int));

            raise!(wr.byte(SVC_SETVIEW));
            let mut viewnum: c_int = 0;
            raise!(w::World_Glue_NumForEdict(
                (*client).edict.cast(),
                &mut viewnum
            ));
            raise!(wr.short(viewnum));

            raise!(wr.byte(SVC_SIGNONNUM));
            raise!(wr.byte(1));
            raise!(wr.flush());

            (*client).sendsignon = PRESPAWN_FLUSH;

            raise!(g::SvMain_Glue_SetupFrames(client.cast()));

            if (*client).message.overflowed && (*client).limit_models > 64 && cantruncate {
                // COMPAT: sv_main.c:618 and :621 test `host_client`.
                let hc = host_client_get();
                if (*hc).protocol_pext2 == 0 || truncated {
                    if (*client).limit_models > (*client).limit_sounds || (*hc).protocol_pext2 != 0
                    {
                        (*client).limit_models /= 2;
                    } else {
                        (*client).limit_sounds /= 2;
                    }
                }
                sz_clear(ptr::addr_of_mut!((*client).message));
                truncated = true;
                continue;
            }
            break;
        }

        if g::NET_CanSendMessage((*client).netconnection.cast())
            && g::NET_SendMessage(
                (*client).netconnection.cast(),
                ptr::addr_of_mut!((*client).message).cast(),
            ) != -1
        {
            sz_clear(ptr::addr_of_mut!((*client).message));
            (*client).last_message = ptr::addr_of_mut!(g::realtime).read();
            (*client).sendsignon = PRESPAWN_DONE;
        }

        if truncated {
            c::Con_Printf(
                c"Protocol limitation (serverinfo) for %s\n".as_ptr(),
                g::NET_QSocketGetTrueAddressString((*client).netconnection.cast()),
            );
        }
        0
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:646 SV_Pext_f

/// `sv_main.c:646` `SV_Pext_f`. `static` in C; reached through the `pext`
/// command `Quake/sv_main_glue.c` registers.
///
/// # Safety
/// Must be called on the main thread with a tokenized command line.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_pext_f() -> Raise {
    // SAFETY: single-threaded engine state.
    unsafe {
        if ptr::addr_of_mut!(c::cvar_cmd::cmd_source).read() != SRC_CLIENT {
            if (*ptr::addr_of_mut!(cls)).state == 0 {
                c::Con_Printf(c"Not connected\n".as_ptr());
                return 0;
            }
            let clp = ptr::addr_of_mut!(cl);
            c::Con_Printf(c"Current Protocols:\n".as_ptr());
            if ((*clp).protocol_pext2 & PEXT2_REPLACEMENTDELTAS) != 0 {
                c::Con_Printf(c"  Replacement Entity Deltas\n".as_ptr());
            }
            if ((*clp).protocol_pext2 & PEXT2_PREDINFO) != 0 {
                c::Con_Printf(c"  Replacement Stats ('predinfo')\n".as_ptr());
            }
            if (*clp).protocol == PROTOCOL_NETQUAKE_U {
                c::Con_Printf(c"  vanilla(15)\n".as_ptr());
            } else if (*clp).protocol == PROTOCOL_FITZQUAKE_U {
                c::Con_Printf(c"  fitzquake(666)\n".as_ptr());
            } else if (*clp).protocol == PROTOCOL_RMQ_U {
                c::Con_Printf(c"  rmq(999)\n".as_ptr());
            } else {
                c::Con_Printf(
                    c"  unknown protocol(%i)\n".as_ptr(),
                    (*clp).protocol as c_int,
                );
            }
            return 0;
        }

        let hc = host_client_get();
        if !(*hc).pextknown && !(*hc).spawned {
            let mut i = 1;
            while i < c::Cmd_Argc() {
                let key = g::strtoul(c::Cmd_Argv(i), ptr::null_mut(), 0) as c_int;
                let value = g::strtoul(c::Cmd_Argv(i + 1), ptr::null_mut(), 0) as c_int;

                if key == PROTOCOL_FTE_PEXT2_I {
                    (*hc).protocol_pext2 = value as c_uint & PEXT2_SUPPORTED_SERVER_U;
                }
                i += 2;
            }

            (*hc).pextknown = true;
            return quake_rs_sv_send_serverinfo(hc.cast());
        }
        0
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:700 SV_ConnectClient

/// `sv_main.c:700` `SV_ConnectClient`.
///
/// Statusized (ADR-009): the body reaches `EDICT_NUM`,
/// `PR_ExecuteProgram (SetNewParms)` and the whole of `SV_SendServerinfo`,
/// so it can return `HOST_GUARD_ABORTSERVER` or `HOST_GUARD_SCREEN_ERROR` as
/// well as `HOST_GUARD_OK`. `Quake/sv_main_glue.c`'s `SV_ConnectClient`
/// wrapper re-raises whatever comes back.
///
/// # Safety
/// `clientnum` must index `svs.clients`; the ambient qcvm must be the
/// server's (ADR-008).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_connect_client(clientnum: c_int) -> Raise {
    // SAFETY: single-threaded engine state; `clientnum < svs.maxclients`.
    unsafe {
        let client = (*svs_p()).clients.add(clientnum as usize);

        if !(*client).netconnection.is_null() {
            c::Con_DPrintf(
                c"Client %s connected\n".as_ptr(),
                g::NET_QSocketGetTrueAddressString((*client).netconnection.cast()),
            );
        } else {
            c::Con_DPrintf(c"Bot connected\n".as_ptr());
        }

        let edictnum = clientnum + 1;
        let mut ent_p: *mut c_void = ptr::null_mut();
        raise!(w::World_Glue_EdictNum(edictnum, &mut ent_p));
        let ent = ent_p.cast::<Edict>();

        let netconnection = (*client).netconnection;
        *ptr::addr_of_mut!(c::net_activeconnections) += 1;

        let mut spawn_parms = [0f32; NUM_TOTAL_SPAWN_PARMS];
        if (*sv_p()).loadgame {
            ptr::copy_nonoverlapping(
                ptr::addr_of!((*client).spawn_parms).cast::<f32>(),
                spawn_parms.as_mut_ptr(),
                NUM_TOTAL_SPAWN_PARMS,
            );
        }
        ptr::write_bytes(client.cast::<u8>(), 0, core::mem::size_of::<Client>());
        (*client).netconnection = netconnection;

        let unconnected = b"unconnected\0";
        ptr::copy_nonoverlapping(
            unconnected.as_ptr().cast::<c_char>(),
            ptr::addr_of_mut!((*client).name).cast::<c_char>(),
            unconnected.len(),
        );
        (*client).active = true;
        (*client).spawned = false;
        (*client).edict = ent;
        (*client).message.data = ptr::addr_of_mut!((*client).msgbuf).cast::<u8>();
        (*client).message.maxsize = MAX_MSGLEN as c_int;
        (*client).message.allowoverflow = true;

        (*client).datagram.data = ptr::addr_of_mut!((*client).datagram_buf).cast::<u8>();
        (*client).datagram.maxsize = MAX_DATAGRAM as c_int;
        (*client).datagram.allowoverflow = true;

        (*client).pextknown = false;
        (*client).protocol_pext2 = 0;

        if (*sv_p()).loadgame {
            ptr::copy_nonoverlapping(
                spawn_parms.as_ptr(),
                ptr::addr_of_mut!((*client).spawn_parms).cast::<f32>(),
                NUM_TOTAL_SPAWN_PARMS,
            );
        } else {
            raise!(g::SvMain_Glue_CallSetNewParms());
            // COMPAT: sv_main.c:750 reads 64 consecutive floats starting at
            // `&pr_global_struct->parm1`, but `globalvars_t` only declares
            // parm1..parm16 -- parms 17..64 come from whatever follows.
            let pgs = ptr::addr_of_mut!(g::pr_global_struct)
                .read()
                .cast::<GlobalVars>();
            let parms = ptr::addr_of!((*pgs).parm1);
            for i in 0..NUM_TOTAL_SPAWN_PARMS {
                (*client).spawn_parms[i] = *parms.add(i);
            }
        }

        quake_rs_sv_send_serverinfo(client.cast())
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:763 SV_CheckForNewClients

/// `sv_main.c:763` `SV_CheckForNewClients`.
///
/// # Safety
/// Must be called on the main thread with the server's qcvm ambient.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_check_for_new_clients() -> Raise {
    // SAFETY: single-threaded engine state.
    unsafe {
        loop {
            let ret = g::NET_CheckNewConnections();
            if ret.is_null() {
                break;
            }

            let mut i = 0;
            while i < (*svs_p()).maxclients {
                if !(*(*svs_p()).clients.add(i as usize)).active {
                    break;
                }
                i += 1;
            }
            if i == (*svs_p()).maxclients {
                // COMPAT: sv_main.c:784 -- the message says
                // "Host_CheckForNewClients" inside SV_CheckForNewClients.
                // Sys_Error terminates; it does not longjmp.
                c::Sys_Error(c"Host_CheckForNewClients: no free clients".as_ptr());
            }

            (*(*svs_p()).clients.add(i as usize)).netconnection = ret.cast();
            raise!(quake_rs_sv_connect_client(i));
        }
        0
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:805 SV_ClearDatagram / :824 SV_ModelIndex / :872 SV_ModelForIndex

/// `sv_main.c:805` `SV_ClearDatagram`. Two field stores; cannot raise, so it
/// is exported plain.
///
/// # Safety
/// Must be called on the main thread.
#[no_mangle]
pub unsafe extern "C" fn SV_ClearDatagram() {
    // SAFETY: single-threaded engine state.
    unsafe { sz_clear(ptr::addr_of_mut!((*sv_p()).datagram)) }
}

/// `sv_main.c:824` `SV_ModelIndex`. The only raise is `Sys_Error`, which
/// terminates, so it is exported plain.
///
/// # Safety
/// `name`, when non-null, must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn SV_ModelIndex(name: *const c_char) -> c_int {
    // SAFETY: single-threaded engine state.
    unsafe {
        if name.is_null() || *name == 0 {
            return 0;
        }

        let mut i: c_int = 0;
        while (i as usize) < MAX_MODELS && !(*sv_p()).model_precache[i as usize].is_null() {
            if g::strcmp((*sv_p()).model_precache[i as usize], name) == 0 {
                return i;
            }
            i += 1;
        }
        if i as usize == MAX_MODELS || (*sv_p()).model_precache[i as usize].is_null() {
            c::Sys_Error(c"SV_ModelIndex: model %s not precached".as_ptr(), name);
        }
        i
    }
}

/// `sv_main.c:872` `SV_ModelForIndex` -- installed as `qcvm->GetModel`.
/// Not declared in `server.h`; exported so the symbol keeps the external
/// linkage the C definition had.
///
/// # Safety
/// Must be called on the main thread.
#[no_mangle]
pub unsafe extern "C" fn SV_ModelForIndex(index: c_int) -> *mut c_void {
    // SAFETY: single-threaded engine state; the index is range-checked.
    unsafe {
        if index < 0 || index as usize >= MAX_MODELS {
            return ptr::null_mut();
        }
        (*sv_p()).models[index as usize].cast()
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:847 SV_SaveSpawnparms

/// `sv_main.c:847` `SV_SaveSpawnparms`.
///
/// # Safety
/// The server qcvm must be ambient (ADR-008).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_save_spawnparms() -> Raise {
    // SAFETY: single-threaded engine state.
    unsafe {
        let pgs = ptr::addr_of_mut!(g::pr_global_struct)
            .read()
            .cast::<GlobalVars>();
        (*svs_p()).serverflags = (*pgs).serverflags as c_int;

        let mut i = 0;
        host_client_set((*svs_p()).clients);
        while i < (*svs_p()).maxclients {
            let hc = host_client_get();
            if (*hc).active {
                raise!(g::SvMain_Glue_CallSetChangeParms((*hc).edict.cast()));
                let parms = ptr::addr_of!((*pgs).parm1);
                let mut j = 0usize;
                while j < NUM_BASIC_SPAWN_PARMS {
                    (*hc).spawn_parms[j] = *parms.add(j);
                    j += 1;
                }
                while j < NUM_TOTAL_SPAWN_PARMS {
                    let mut v: c_float = 0.0;
                    g::SvMain_Glue_SpawnParmGlobal(j as c_int + 1, &mut v);
                    (*hc).spawn_parms[j] = v;
                    j += 1;
                }
            }
            i += 1;
            host_client_set(host_client_get().add(1));
        }
        0
    }
}

// ---------------------------------------------------------------------------
// sv_main.c:886 SV_SpawnServer

/// `sv_main.c:886` `SV_SpawnServer`.
///
/// # Safety
/// `server` must be NUL-terminated; must be called on the main thread.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sv_spawn_server(server: *const c_char) -> Raise {
    // SAFETY: single-threaded engine state; the qcvm is switched exactly as
    // in C and re-read after every switch (ADR-008).
    unsafe {
        let saved_vm = ptr::addr_of_mut!(c::qcvm).read();

        if *ptr::addr_of!(g::hostname.string).read() == 0 {
            raise!(g::SvMain_Glue_CvarSet(
                c"hostname".as_ptr(),
                c"UNNAMED".as_ptr()
            ));
        }
        g::SCR_CenterPrintClear();

        c::Con_DPrintf(c"SpawnServer: %s\n".as_ptr(), server);
        (*svs_p()).changelevel_issued = false;

        g::PR_SwitchQCVM(ptr::null_mut());

        if (*sv_p()).active {
            raise!(g::SvMain_Glue_SendReconnect());
        }

        if cvar_value(ptr::addr_of!(g::coop)) != 0.0 {
            raise!(g::SvMain_Glue_CvarSet(
                c"deathmatch".as_ptr(),
                c"0".as_ptr()
            ));
        }
        // C promotes skill.value to double before adding the 0.5 literal.
        let skill_now = ((cvar_value(ptr::addr_of!(g::skill)) as f64 + 0.5) as c_int).clamp(0, 3);
        ptr::addr_of_mut!(g::current_skill).write(skill_now);

        raise!(g::SvMain_Glue_CvarSetValue(
            c"skill".as_ptr(),
            skill_now as c_float
        ));

        raise!(g::SvMain_Glue_ClearMemory());

        q_strlcpy(
            ptr::addr_of_mut!((*sv_p()).name).cast::<c_char>(),
            server,
            64,
        );

        let protocol = ptr::addr_of_mut!(g::sv_protocol).read();
        (*sv_p()).protocol = protocol as c_uint;

        if (*sv_p()).protocol == PROTOCOL_RMQ_U {
            if ptr::addr_of_mut!(g::sv_protocol_pext2).read() != 0 {
                (*sv_p()).protocolflags = PRFL_FLOATCOORD | PRFL_SHORTANGLE;
            } else {
                (*sv_p()).protocolflags = PRFL_INT32COORD | PRFL_SHORTANGLE;
            }
        } else {
            (*sv_p()).protocolflags = 0;
        }

        g::PR_SwitchQCVM(saved_vm.cast());
        raise!(g::SvMain_Glue_LoadProgs());

        let max_edicts_value = cvar_value(ptr::addr_of!(g::max_edicts)) as c_int;
        (*vm()).max_edicts = max_edicts_value.clamp(MIN_EDICTS, MAX_EDICTS);
        let bytes = (*vm()).max_edicts.wrapping_mul((*vm()).edict_size) as usize;
        (*vm()).edicts = c::Mem_Alloc(bytes).cast::<Edict>();

        g::SvMain_Glue_InitDebugEdicts();

        (*sv_p()).datagram.maxsize = MAX_DATAGRAM as c_int;
        (*sv_p()).datagram.cursize = 0;
        (*sv_p()).datagram.data = ptr::addr_of_mut!((*sv_p()).datagram_buf).cast::<u8>();

        (*sv_p()).multicast.maxsize = MAX_DATAGRAM as c_int;
        (*sv_p()).multicast.cursize = 0;
        (*sv_p()).multicast.data = ptr::addr_of_mut!((*sv_p()).multicast_buf).cast::<u8>();

        (*sv_p()).reliable_datagram.maxsize = MAX_DATAGRAM as c_int;
        (*sv_p()).reliable_datagram.cursize = 0;
        (*sv_p()).reliable_datagram.data =
            ptr::addr_of_mut!((*sv_p()).reliable_datagram_buf).cast::<u8>();

        (*sv_p()).signon.maxsize = (MAX_MSGLEN - 2) as c_int;
        (*sv_p()).signon.cursize = 0;
        (*sv_p()).signon.data = ptr::addr_of_mut!((*sv_p()).signon_buf).cast::<u8>();

        (*vm()).reserved_edicts = (*svs_p()).maxclients + 1;
        (*vm()).num_edicts = (*vm()).reserved_edicts;

        for i in 0..(*svs_p()).maxclients {
            let mut ent_p: *mut c_void = ptr::null_mut();
            raise!(w::World_Glue_EdictNum(i + 1, &mut ent_p));
            g::SvMain_Glue_AssertEdictNotFree(ent_p);
            (*(*svs_p()).clients.add(i as usize)).edict = ent_p.cast::<Edict>();
        }

        (*sv_p()).state = SS_LOADING;
        (*sv_p()).paused = false;
        (*sv_p()).nomonsters = cvar_value(ptr::addr_of!(g::nomonsters)) != 0.0;

        (*vm()).time = 1.0;

        q_strlcpy(
            ptr::addr_of_mut!((*sv_p()).name).cast::<c_char>(),
            server,
            64,
        );
        format_map_name(
            ptr::addr_of_mut!((*sv_p()).modelname).cast::<c_char>(),
            64,
            server,
        );

        let mut world_p: *mut c_void = ptr::null_mut();
        raise!(g::SvMain_Glue_ModForName(
            ptr::addr_of!((*sv_p()).modelname).cast::<c_char>(),
            &mut world_p
        ));
        (*vm()).worldmodel = world_p;
        if world_p.is_null() || (*world_p.cast::<QModel>()).type_ != MOD_BRUSH {
            c::Con_Printf(
                c"Couldn't spawn server %s\n".as_ptr(),
                ptr::addr_of!((*sv_p()).modelname).cast::<c_char>(),
            );
            (*sv_p()).active = false;
            return 0;
        }
        let world = world_p.cast::<QModel>();
        (*sv_p()).models[1] = world;
        (*vm()).get_model = Some(SV_ModelForIndex);

        crate::world::SV_ClearWorld();

        let dummy = ptr::addr_of_mut!(DUMMY).cast::<c_char>();
        (*sv_p()).sound_precache[0] = dummy;
        (*sv_p()).model_precache[0] = dummy;
        (*sv_p()).model_precache[1] = ptr::addr_of!((*sv_p()).modelname).cast::<c_char>();
        if (*world).numsubmodels > MAX_MODELS as i32 {
            c::Con_Printf(
                c"too many inline models %s\n".as_ptr(),
                ptr::addr_of!((*sv_p()).modelname).cast::<c_char>(),
            );
            (*sv_p()).active = false;
            return 0;
        }
        let localmodels = ptr::addr_of_mut!(LOCALMODELS).cast::<[c_char; 8]>();
        for i in 1..(*world).numsubmodels {
            let name = (*localmodels.add(i as usize)).as_ptr();
            (*sv_p()).model_precache[(1 + i) as usize] = name;
            let mut m: *mut c_void = ptr::null_mut();
            raise!(g::SvMain_Glue_ModForName(name, &mut m));
            (*sv_p()).models[(i + 1) as usize] = m.cast::<QModel>();
        }

        let mut ent_p: *mut c_void = ptr::null_mut();
        raise!(w::World_Glue_EdictNum(0, &mut ent_p));
        let ent = ent_p.cast::<Edict>();
        ptr::write_bytes(
            ptr::addr_of_mut!((*ent).v).cast::<u8>(),
            0,
            ((*(*vm()).progs).entityfields * 4) as usize,
        );
        (*ent).free = false;
        (*ent).v.model = g::PR_SetEngineString(ptr::addr_of!((*world).name).cast::<c_char>());
        (*ent).v.modelindex = 1.0;
        (*ent).v.solid = SOLID_BSP;
        (*ent).v.movetype = MOVETYPE_PUSH;

        let pgs = ptr::addr_of_mut!(g::pr_global_struct)
            .read()
            .cast::<GlobalVars>();
        let coop_value = cvar_value(ptr::addr_of!(g::coop));
        if coop_value != 0.0 {
            (*pgs).coop = coop_value;
        } else {
            (*pgs).deathmatch = cvar_value(ptr::addr_of!(g::deathmatch));
        }

        (*pgs).mapname = g::PR_SetEngineString(ptr::addr_of!((*sv_p()).name).cast::<c_char>());
        (*pgs).serverflags = (*svs_p()).serverflags as f32;

        raise!(g::SvMain_Glue_LoadFromFile((*world).entities));

        (*sv_p()).active = true;

        raise!(g::SvMain_Glue_PrecacheModel(c"progs/player.mdl".as_ptr()));

        (*sv_p()).state = SS_ACTIVE;

        ptr::addr_of_mut!(c::host_frametime).write(0.1);
        raise!(crate::sv_phys::quake_rs_sv_physics());
        raise!(crate::sv_phys::quake_rs_sv_physics());

        raise!(g::SvMain_Glue_CreateBaseline());

        if (*sv_p()).signon.cursize > 8000 - 2 {
            c::Con_DWarning(
                c"%i byte signon buffer exceeds standard limit of 7998 (max = %d).\n".as_ptr(),
                (*sv_p()).signon.cursize,
                (*sv_p()).signon.maxsize,
            );
        }

        let mut i = 0;
        host_client_set((*svs_p()).clients);
        while i < (*svs_p()).maxclients {
            let hc = host_client_get();
            (*hc).knowntoqc = false;
            if (*hc).active {
                raise!(quake_rs_sv_send_serverinfo(hc.cast()));
            }
            i += 1;
            host_client_set(host_client_get().add(1));
        }

        c::Con_DPrintf(c"Server spawned.\n".as_ptr());
        0
    }
}
