//! `Quake/host.c` -- the frame loop and engine bring-up (Rust migration
//! Phase 7 M8, T8.2).
//!
//! Near-transliteration of everything `host.c` defined except the raise
//! machinery: `Host_EndGame`, `Host_Error`, `Host_Guard` and `Host_Reraise`
//! own the `setjmp` shell and stay C in `Quake/host_glue.c` until Phase 9
//! (ADR-009). `_Host_Frame`'s `setjmp` likewise stays there, as
//! `Host_Glue_FrameInner`; the rest of that function is
//! [`quake_rs_host_frame_core`].
//!
//! ADR-007: no dual-view row opens or closes here. Every C-visible object
//! `host.c` defined keeps C storage in the glue -- the two `jmp_buf`s are the
//! longjmp targets, `host_client` and the cvars are addressed from fourteen
//! other translation units, and `dev_stats`/`dev_peakstats`/`dev_overflows`
//! are shared with `cl_parse.c` and `sv_send.c`. The only objects that move
//! are the eleven function-scope `static`s below, which had internal linkage.
//!
//! ADR-008: `_Host_Frame`'s VM-switch ordering is preserved exactly -- the
//! server tick inside `PR_SwitchQCVM (&sv.qcvm)` / `PR_SwitchQCVM (NULL)`,
//! then the CSQC physics tick inside `PR_SwitchQCVM (&cl.qcvm)` /
//! `PR_SwitchQCVM (NULL)`. `PR_SwitchQCVM` is called through: it assigns two
//! globals and reaches only `Sys_Error`.
//!
//! ADR-009 audit. `host.c` has no `Host_Error`/`Host_EndGame` call site of its
//! own, so there is no file-local raise-code set: every core returns a
//! `Host_Guard` status verbatim. The guarded set is enumerated in
//! `Quake/host_glue.c`. The plain set is everything else, of which the
//! interesting members are `Con_Printf`/`Con_DPrintf`/`Con_Warning`/
//! `Con_DWarning` (issued as C variadics, per `cl_demo_glue.c:47-51`),
//! `PR_SwitchQCVM`, `Cvar_RegisterVariable`, `Cvar_SetCallback`,
//! `COM_CheckParm`, `COM_Rand`, `COM_FOpenPrefFile`, `COM_SkipPath`,
//! `COM_StripExtension`, `Sys_DoubleTime`, `Sys_Printf`, `Sys_Error`,
//! `Sys_ConsoleInput`, `Sys_SendKeyEvents`, `Mem_Alloc`, `Mem_Free`,
//! `Info_GetKey`, `Tasks_IsWorker`, `Cbuf_Waited`, `SDL_Delay`, the
//! `Steam_SetStatus_*` trio and the `stdio` calls in
//! `Host_WriteConfiguration`.
//!
//! One raise-topology detail: where `host.c` called `SV_DropClient`,
//! `Host_ServerFrame`, `Host_FilterTime`, `Host_GetConsoleCommands` or
//! `SV_BroadcastPrintf` -- all of which are now C glue wrappers that
//! `Host_Reraise` -- the port calls the Rust core directly and propagates the
//! status. Routing through the C wrapper would longjmp across this Rust frame.
//!
//! ADR-005: the specifiers reachable from this file are `%s`, `%i`, `%u`,
//! `%d`, `%x`, `%2i`, `%1.2f`, `%5.2f`, `%3d`, `%.3f` and `%.0f`, all of them
//! in `Con_*`/`Sys_Printf`/`fprintf` calls issued as C variadics. There is no
//! `%g`, `%e` or `%a`, and nothing here formats through the Rust formatter, so
//! its panic path is unreachable. The three places `host.c` used
//! `q_vsnprintf`/`q_snprintf` to build a buffer are reproduced by the
//! byte-copying helpers below, which match `vsnprintf` truncation exactly.
//!
//! ADR-010: the arithmetic here is plain add/multiply/compare with no libm
//! call; the `CLAMP`s are transliterated in the C operand order and width,
//! including the two places C narrows a `double` expression into a `float`.

#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_main as gcl;
use quake_c_sys::cl_parse as gcp;
use quake_c_sys::host as g;
use quake_c_sys::progs_builtins_sv as gpb;
use quake_c_sys::sv_main as gsv;
use quake_c_sys::sv_phys as gsp;
use quake_c_sys::sv_user as gsu;
use quake_types::host::{
    Client, ClientState, ClientStatic, Server, ServerStatic, CA_CONNECTED, CA_DEDICATED,
    CA_DISCONNECTED, MAX_PARTICLETYPES,
};
use quake_types::net::SizeBuf;
use quake_types::progs::{Edict, GlobalVars, QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2};

use crate::cl_main::{cl, cls};
use crate::sv_main::{sv, svs};

// ---------------------------------------------------------------------------
// ADR-009 plumbing.

/// A `Host_Guard` status: 0 = returned normally, 1 = `Host_Error` /
/// `Host_EndGame`, 2 = `screen_error`. `host.c` raises nothing itself, so
/// there are no file-local codes above those three.
type Raise = c_int;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// engine constants

/// `quakedef.h` -- the user config filename.
const CONFIG_NAME: &core::ffi::CStr = c"vkQuake.cfg";
/// `quakedef.h` -- `MAX_PHYSICS_FREQ`.
const MAX_PHYSICS_FREQ: f64 = 72.0;
/// `quakedef.h` -- `HOST_NETITERVAL_FREQ`.
const HOST_NETITERVAL_FREQ: f64 = 71.9990;
/// `quakedef.h` -- `MAX_SCOREBOARD`.
const MAX_SCOREBOARD: c_int = 16;
/// `q_types.h:240` -- `MAX_QPATH`.
const MAX_QPATH: usize = 64;
/// `quakedef.h` -- `STAT_TOTALSECRETS` / `STAT_TOTALMONSTERS`.
const STAT_TOTALSECRETS: usize = 11;
const STAT_TOTALMONSTERS: usize = 12;
/// `client.h` -- `SIGNONS`.
const SIGNONS: c_int = 4;
/// `quakedef.h` -- the edict-count clamp `CL_LoadCSProgs` applies.
const MIN_EDICTS: c_int = 256;
const MAX_EDICTS: c_int = 32000;
/// `server.h:248` -- `SOLID_BSP`.
const SOLID_BSP: f32 = 4.0;
/// `protocol.h` -- `GAME_COOP`.
const GAME_COOP: c_int = 0;
/// `progs.h` -- `PROGHEADER_CRC`.
const PROGHEADER_CRC: c_uint = 5927;
/// `quakever.h:41` / `:39` -- `VKQUAKE_VERSION` and its patch level.
const VKQUAKE_VERSION: f64 = 1.0;
const VKQUAKE_VER_PATCH: c_int = 0;
/// `protocol.h` service bytes `host.c` writes.
const SVC_PRINT: c_int = 8;
const SVC_STUFFTEXT: c_int = 9;
const SVC_DISCONNECT: c_int = 2;
const SVC_UPDATENAME: c_int = 13;
const SVC_UPDATEFRAGS: c_int = 14;
const SVC_UPDATECOLORS: c_int = 17;
/// `Quake/keys.h:138` -- `key_game`, the first `keydest_t` enumerator.
const KEY_GAME: c_int = 0;
/// `protocol.h` -- `clc_stringcmd`.
const CLC_STRINGCMD: c_int = 4;
/// `Quake/host_glue.c` -- `host_write_t.kind`.
const WRITE_BYTE: c_int = 0;
const WRITE_SHORT: c_int = 1;
const WRITE_STRING: c_int = 2;

// ---------------------------------------------------------------------------
// Rust-owned storage: host.c's function-scope statics. They had internal
// linkage in C, so the port owns them outright and no ADR-007 row is involved.

/// `host.c:848-849` -- `Host_ServerFrame`'s sv_speeds accumulators.
static mut SVF_CLIENTS_MS: f64 = 0.0;
static mut SVF_PHYSICS_MS: f64 = 0.0;
static mut SVF_STATS_MS: f64 = 0.0;
static mut SVF_SEND_MS: f64 = 0.0;
static mut SVF_INTERVAL_START: f64 = 0.0;
static mut SVF_TICKS: c_int = 0;

/// `host.c:1032-1034` -- `Host_UpdateSteamStatus`'s change detector.
/// `lastmap` is `char[sizeof (cl.levelname)]`, i.e. 128 bytes.
static mut STEAM_NEXTUPDATE: f64 = 0.0;
static mut STEAM_LASTMAP: [c_char; 128] = [0; 128];
static mut STEAM_LASTPLAYERS: c_int = -1;
static mut STEAM_LASTMAXPLAYERS: c_int = -1;

/// `host.c:1087-1090` -- `_Host_Frame`'s accumulator and host_speeds stamps.
static mut FRAME_ACCUMTIME: f64 = 0.0;
static mut FRAME_TIME1: f64 = 0.0;
static mut FRAME_TIME2: f64 = 0.0;
static mut FRAME_TIME3: f64 = 0.0;

/// `host.c:1239-1240` -- `Host_Frame`'s serverprofile accumulators.
static mut PROFILE_TIMETOTAL: f64 = 0.0;
static mut PROFILE_TIMECOUNT: c_int = 0;

/// `host.c:1419` -- `Host_Shutdown`'s recursion guard.
static mut SHUTDOWN_ISDOWN: bool = false;

// ---------------------------------------------------------------------------
// small helpers

#[inline]
fn sv_p() -> *mut Server {
    ptr::addr_of_mut!(sv)
}

#[inline]
fn svs_p() -> *mut ServerStatic {
    ptr::addr_of_mut!(svs)
}

#[inline]
fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl)
}

#[inline]
fn cls_p() -> *mut ClientStatic {
    ptr::addr_of_mut!(cls)
}

/// The ambient qcvm (ADR-008).
#[inline]
unsafe fn vm() -> *mut QcVm {
    // SAFETY: single-threaded engine state.
    unsafe { ptr::addr_of_mut!(c::qcvm).read().cast::<QcVm>() }
}

/// `pr_global_struct`, as the mirror. Re-read after every `PR_SwitchQCVM`.
#[inline]
unsafe fn pgs() -> *mut GlobalVars {
    // SAFETY: the same storage as `qcvm->globals`; the VM is selected by the
    // caller, exactly as in C.
    unsafe {
        ptr::addr_of_mut!(gsv::pr_global_struct)
            .read()
            .cast::<GlobalVars>()
    }
}

extern "C" {
    /// `Quake/host_glue.c` -- the client slot the current command is attributed
    /// to. Declared here rather than in `quake-c-sys` because it is typed with
    /// the ADR-011 mirror `Client`, matching `sv_main.rs` and `sv_send.rs`.
    static mut host_client: *mut Client;
}

/// `cmd.h:110` -- `Cmd_AddCommand (name, func)` expands to
/// `Cmd_AddCommand2 (name, func, src_command, false)`.
unsafe fn add_command(name: *const c_char, func: Option<unsafe extern "C" fn()>) {
    // SAFETY: `name` is NUL-terminated and `func` has the `xcommand_t` signature.
    unsafe {
        c::Cmd_AddCommand2(name, func, c::cmd_source_t_src_command, false);
    }
}

/// The current value of the C global `host_client`.
#[inline]
unsafe fn host_client_get() -> *mut Client {
    // SAFETY: single-threaded engine state.
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
    // SAFETY: `var` always points at an engine-owned `cvar_t`.
    unsafe { ptr::addr_of!((*var).value).read() }
}

/// `CLAMP (_minval, x, _maxval)` in `double`, the width C's macro produces
/// whenever either bound is a `double` literal.
#[inline]
fn clamp_f64(minval: f64, x: f64, maxval: f64) -> f64 {
    if x < minval {
        minval
    } else if x > maxval {
        maxval
    } else {
        x
    }
}

/// `CLAMP` over `int`, as `CL_LoadCSProgs` uses it.
#[inline]
fn clamp_i32(minval: c_int, x: c_int, maxval: c_int) -> c_int {
    if x < minval {
        minval
    } else if x > maxval {
        maxval
    } else {
        x
    }
}

/// `common.c` `q_strlcpy` -- truncating bounded copy, always NUL-terminated.
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

/// `strcmp (a, b) == 0` over two NUL-terminated C strings.
unsafe fn c_str_eq(a: *const c_char, b: *const c_char) -> bool {
    // SAFETY: both operands are NUL-terminated buffers owned by the caller.
    unsafe {
        let mut i = 0usize;
        loop {
            let (x, y) = (*a.add(i), *b.add(i));
            if x != y {
                return false;
            }
            if x == 0 {
                return true;
            }
            i += 1;
        }
    }
}

/// A truncating byte writer over a fixed C buffer, reproducing `vsnprintf`'s
/// contract (at most `len - 1` payload bytes, always NUL-terminated). The
/// three `q_vsnprintf`/`q_snprintf` sites in `host.c` only ever interpolate
/// `%s` and one `%x`, so this replaces the formatter entirely and ADR-005's
/// `%g`/`%e` panic stays unreachable.
struct CBuf {
    p: *mut c_char,
    len: usize,
    used: usize,
}

impl CBuf {
    /// # Safety
    /// `p` must point at `len` writable bytes.
    unsafe fn new(p: *mut c_char, len: usize) -> Self {
        let mut b = CBuf { p, len, used: 0 };
        if len != 0 {
            // SAFETY: `len != 0`, so byte 0 is in bounds.
            unsafe { *b.p = 0 };
        }
        b.used = 0;
        b
    }

    fn push(&mut self, ch: c_char) {
        if self.len == 0 || self.used + 1 >= self.len {
            return;
        }
        // SAFETY: bounded by the check above; the NUL always fits after it.
        unsafe {
            *self.p.add(self.used) = ch;
            self.used += 1;
            *self.p.add(self.used) = 0;
        }
    }

    fn lit(&mut self, s: &[u8]) {
        for &b in s {
            self.push(b as c_char);
        }
    }

    /// `%s`
    ///
    /// # Safety
    /// `s` must be NUL-terminated (or null, which C's `%s` would not accept
    /// but no call site here can produce).
    unsafe fn cstr(&mut self, s: *const c_char) {
        // SAFETY: caller contract.
        unsafe {
            let mut i = 0usize;
            while *s.add(i) != 0 {
                self.push(*s.add(i));
                i += 1;
            }
        }
    }

    /// `%x` -- lowercase hex, no padding, `0` for zero.
    fn hex(&mut self, mut v: c_uint) {
        let mut digits = [0u8; 8];
        let mut n = 0usize;
        if v == 0 {
            self.push(b'0' as c_char);
            return;
        }
        while v != 0 {
            digits[n] = b"0123456789abcdef"[(v & 0xf) as usize];
            v >>= 4;
            n += 1;
        }
        while n != 0 {
            n -= 1;
            self.push(digits[n] as c_char);
        }
    }
}

/// `SZ_Clear (sb)`.
#[inline]
unsafe fn sz_clear(sb: *mut SizeBuf) {
    // SAFETY: `sb` is a live engine `sizebuf_t`.
    unsafe { c::cvar_cmd::SZ_Clear(sb.cast()) }
}

/// One guarded `MSG_Write*` batch against `sb` (`Quake/host_glue.c`). Ops
/// replay in insertion order inside a single `Host_Guard`, so both the byte
/// stream and the raise point are what the individual C calls produced.
#[inline]
unsafe fn write_batch(sb: *mut c_void, ops: &[g::HostWriteOp]) -> Raise {
    // SAFETY: `sb` is a live engine `sizebuf_t`; every `p` outlives the call.
    unsafe { g::Host_Glue_WriteBatch(sb, ops.as_ptr(), ops.len() as c_int) }
}

#[inline]
fn op_byte(v: c_int) -> g::HostWriteOp {
    g::HostWriteOp {
        kind: WRITE_BYTE,
        i: v,
        p: ptr::null(),
    }
}

#[inline]
fn op_short(v: c_int) -> g::HostWriteOp {
    g::HostWriteOp {
        kind: WRITE_SHORT,
        i: v,
        p: ptr::null(),
    }
}

#[inline]
fn op_string(s: *const c_char) -> g::HostWriteOp {
    g::HostWriteOp {
        kind: WRITE_STRING,
        i: 0,
        p: s.cast(),
    }
}

// ---------------------------------------------------------------------------
// host.c:115-176 -- the three cvar callbacks.
//
// All three are `static` in C and reach only `Con_Printf`/`Con_Warning`, which
// this file treats as raise-free (ADR-009 rule 4), so they cross the FFI as
// plain Rust `extern "C"` callbacks rather than glue trampolines -- the
// precedent is `snd_dma.rs:258-265`.

/// `host.c:115` -- `Max_Edicts_f`.
unsafe extern "C" fn max_edicts_f(_var: *mut c::cvar_t) {
    // SAFETY: called from the C cvar registry on the main thread.
    unsafe {
        if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED
            || ptr::addr_of!((*sv_p()).active).read()
        {
            c::Con_Printf(
                c"Changes to max_edicts will not take effect until the next time a map is loaded.\n"
                    .as_ptr(),
            );
        }
    }
}

/// `host.c:131` -- `Max_Fps_f`.
unsafe extern "C" fn max_fps_f(var: *mut c::cvar_t) {
    // SAFETY: `var` is the registry's `cvar_t`.
    unsafe {
        // host_phys_max_ticrate overrides normal behaviour
        if cvar_value(ptr::addr_of!(g::host_phys_max_ticrate)) > 0.0 {
            phys_ticrate_f(ptr::addr_of_mut!(g::host_phys_max_ticrate));
            return;
        }

        let v = cvar_value(var) as f64;
        if v > MAX_PHYSICS_FREQ || v <= 0.0 {
            if ptr::addr_of!(gcl::host_netinterval).read() == 0.0 {
                c::Con_Printf(c"Using renderer/network isolation.\n".as_ptr());
            }
            ptr::addr_of_mut!(gcl::host_netinterval).write((1.0 / HOST_NETITERVAL_FREQ) as c_float);
        } else {
            if ptr::addr_of!(gcl::host_netinterval).read() != 0.0 {
                c::Con_Printf(c"Disabling renderer/network isolation.\n".as_ptr());
            }
            ptr::addr_of_mut!(gcl::host_netinterval).write(0.0);

            if v > MAX_PHYSICS_FREQ {
                c::Con_Warning(c"host_maxfps above 72 breaks physics.\n".as_ptr());
            }
        }
    }
}

/// `host.c:162` -- `Phys_Ticrate_f`.
unsafe extern "C" fn phys_ticrate_f(var: *mut c::cvar_t) {
    // SAFETY: `var` is the registry's `cvar_t`.
    unsafe {
        if cvar_value(var) > 0.0 {
            // clamp within valid limits, authorize float values.
            // C's CLAMP widens to double here (both bounds are double
            // literals) and narrows once on the store back into `.value`.
            let clamped = clamp_f64(0.0, cvar_value(var) as f64, MAX_PHYSICS_FREQ) as c_float;
            ptr::addr_of_mut!((*var).value).write(clamped);

            c::Con_Printf(
                c"Using max physics tics rate = %dHz.\n".as_ptr(),
                clamped as c_int,
            );
            ptr::addr_of_mut!(gcl::host_netinterval).write((1.0 / clamped as f64) as c_float);
        } else {
            c::Con_Printf(
                c"Disable max physics tics rate, using host_maxfps control...\n".as_ptr(),
            );
            // apply max_fps policy
            max_fps_f(ptr::addr_of_mut!(g::host_maxfps));
        }
    }
}

// ---------------------------------------------------------------------------
// host.c:357 -- Host_FindMaxClients

#[no_mangle]
pub extern "C" fn quake_rs_host_find_max_clients() -> Raise {
    // SAFETY: engine bring-up, single-threaded.
    unsafe {
        let svsp = svs_p();
        let clsp = cls_p();

        ptr::addr_of_mut!((*svsp).maxclients).write(1);

        let argc = ptr::addr_of!(c::com_argc).read();
        let argv = ptr::addr_of!(c::com_argv).read();

        let mut i = c::COM_CheckParm(c"-dedicated".as_ptr());
        if i != 0 {
            ptr::addr_of_mut!((*clsp).state).write(CA_DEDICATED);
            if i != argc - 1 {
                ptr::addr_of_mut!((*svsp).maxclients)
                    .write(g::atoi(*argv.offset((i + 1) as isize)));
            } else {
                ptr::addr_of_mut!((*svsp).maxclients).write(8);
            }
        } else {
            ptr::addr_of_mut!((*clsp).state).write(CA_DISCONNECTED);
        }

        i = c::COM_CheckParm(c"-listen".as_ptr());
        if i != 0 {
            if ptr::addr_of!((*clsp).state).read() == CA_DEDICATED {
                c::Sys_Error(c"Only one of -dedicated or -listen can be specified".as_ptr());
            }
            if i != argc - 1 {
                ptr::addr_of_mut!((*svsp).maxclients)
                    .write(g::atoi(*argv.offset((i + 1) as isize)));
            } else {
                ptr::addr_of_mut!((*svsp).maxclients).write(8);
            }
        }

        let mc = ptr::addr_of!((*svsp).maxclients).read();
        if mc < 1 {
            ptr::addr_of_mut!((*svsp).maxclients).write(8);
        } else if mc > MAX_SCOREBOARD {
            ptr::addr_of_mut!((*svsp).maxclients).write(MAX_SCOREBOARD);
        }

        ptr::addr_of_mut!((*svsp).maxclientslimit).write(MAX_SCOREBOARD);
        let bytes =
            ptr::addr_of!((*svsp).maxclientslimit).read() as usize * core::mem::size_of::<Client>();
        ptr::addr_of_mut!((*svsp).clients).write(c::Mem_Alloc(bytes).cast::<Client>());

        if ptr::addr_of!((*svsp).maxclients).read() > 1 {
            raise!(g::Host_Glue_CvarSetQuick(
                ptr::addr_of_mut!(gsv::deathmatch).cast(),
                c"1".as_ptr()
            ));
        } else {
            raise!(g::Host_Glue_CvarSetQuick(
                ptr::addr_of_mut!(gsv::deathmatch).cast(),
                c"0".as_ptr()
            ));
        }
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:401 -- Host_Version_f

#[no_mangle]
pub extern "C" fn quake_rs_host_version_f() -> Raise {
    // SAFETY: `Con_Printf` is a C variadic; every argument below is a `%s`
    // operand pointing at a static C string, or the `double` VERSION.
    unsafe {
        c::Con_Printf(
            c"Quake Version %1.2f\n".as_ptr(),
            ptr::addr_of!(g::host_glue_version).read(),
        );
        // C concatenated the version strings into the format literal. They
        // contain no '%', so routing them through a "%s" operand produces
        // byte-identical output.
        c::Con_Printf(
            c"QuakeSpasm Version %s\n".as_ptr(),
            ptr::addr_of!(g::host_glue_quakespasm_ver).read(),
        );
        c::Con_Printf(
            c"vkqr-engine Version %s\n".as_ptr(),
            ptr::addr_of!(g::host_glue_engine_ver).read(),
        );
        c::Con_Printf(
            c"Exe: %s %s %s\n".as_ptr(),
            ptr::addr_of!(g::host_glue_build_time).read(),
            ptr::addr_of!(g::host_glue_build_date).read(),
            ptr::addr_of!(g::host_glue_build_suffix).read(),
        );
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:416 -- Host_Callback_Notify

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_host_callback_notify(var: *mut c::cvar_t) -> Raise {
    // SAFETY: `var` is the registry's `cvar_t`; the buffer is stack-local.
    unsafe {
        if ptr::addr_of!((*sv_p()).active).read() {
            // `SV_BroadcastPrintf` is a C variadic wrapper that re-raises, so
            // it cannot be called from this Rust frame. Format exactly as its
            // internal `q_vsnprintf (string, sizeof (string), ...)` would
            // (1024 bytes, truncating, always NUL-terminated) and hand the
            // result to the Rust core.
            let mut buf = [0 as c_char; 1024];
            let mut w = CBuf::new(buf.as_mut_ptr(), buf.len());
            w.lit(b"\"");
            w.cstr(ptr::addr_of!((*var).name).read());
            w.lit(b"\" changed to \"");
            w.cstr(ptr::addr_of!((*var).string).read());
            w.lit(b"\"\n");
            raise!(sv_broadcast_printf_core(buf.as_ptr()));
        }
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:427 -- Host_InitLocal

#[no_mangle]
pub extern "C" fn quake_rs_host_init_local() -> Raise {
    // SAFETY: engine bring-up, single-threaded.
    unsafe {
        // The registry must receive the C glue wrapper: a raise inside
        // `Host_Version_f` has to unwind through a C frame (ADR-009 rule 3).
        add_command(c"version".as_ptr(), Some(g::Host_Version_f));

        raise!(g::Host_Glue_Host_InitCommands());

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::pr_engine));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::host_framerate));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::host_speeds));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(gsp::sv_speeds));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::host_maxfps)); // johnfitz
        c::Cvar_SetCallback(ptr::addr_of_mut!(g::host_maxfps), Some(max_fps_f));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::host_phys_max_ticrate)); // vso
        c::Cvar_SetCallback(
            ptr::addr_of_mut!(g::host_phys_max_ticrate),
            Some(phys_ticrate_f),
        );
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::host_timescale)); // johnfitz

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::cl_nocsqc)); // spike
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(gsv::max_edicts)); // johnfitz
        c::Cvar_SetCallback(ptr::addr_of_mut!(gsv::max_edicts), Some(max_edicts_f));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::devstats)); // johnfitz

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::sys_ticrate));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::serverprofile));

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::fraglimit));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::timelimit));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(gpb::teamplay));
        c::Cvar_SetCallback(
            ptr::addr_of_mut!(g::fraglimit),
            Some(g::Host_Callback_Notify),
        );
        c::Cvar_SetCallback(
            ptr::addr_of_mut!(g::timelimit),
            Some(g::Host_Callback_Notify),
        );
        c::Cvar_SetCallback(
            ptr::addr_of_mut!(gpb::teamplay),
            Some(g::Host_Callback_Notify),
        );
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::samelevel));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::noexit));
        c::Cvar_SetCallback(ptr::addr_of_mut!(g::noexit), Some(g::Host_Callback_Notify));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(gsv::skill));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(c::developer));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(gsv::coop));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(gsv::deathmatch));

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::campaign));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::horde));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::sv_cheats));

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::pausable));

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::autoload));
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::autofastload));

        c::Cvar_RegisterVariable(ptr::addr_of_mut!(g::temp1));

        raise!(quake_rs_host_find_max_clients());
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:486 -- Host_WriteConfiguration

#[no_mangle]
pub extern "C" fn quake_rs_host_write_configuration() -> Raise {
    // SAFETY: engine state; `f` is a libc FILE the callees only append to.
    unsafe {
        // dedicated servers initialize the host but don't parse and set the
        // config cvars
        if ptr::addr_of!(c::cvar_cmd::host_initialized).read()
            && !ptr::addr_of!(c::isDedicated).read()
            && ptr::addr_of!((*ptr::addr_of!(g::host_parms).read()).errstate).read() == 0
        {
            let f = c::COM_FOpenPrefFile(CONFIG_NAME.as_ptr(), c"w".as_ptr());
            if f.is_null() {
                c::Con_Printf(c"Couldn't write vkQuake.cfg.\n".as_ptr());
                return g::HOST_GUARD_OK;
            }

            // VID_SyncCvars (); //johnfitz -- write actual current mode to
            // config file, in case cvars were messed with

            // A raise inside either writer leaves `f` open, exactly as the C
            // longjmp out of this function did.
            raise!(g::Host_Glue_Key_WriteBindings(f.cast()));
            raise!(g::Host_Glue_Cvar_WriteVariables(f.cast()));

            // johnfitz -- extra commands to preserve state
            c::cvar_cmd::fprintf(f, c"vid_restart\n".as_ptr());
            // always enable mouse look on config, can be overriden by -mlook
            // in autoexec.cfg
            c::cvar_cmd::fprintf(f, c"+mlook\n".as_ptr());
            // johnfitz

            c::stdio::fclose(f);
        }
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:522, :542, :569 -- the three variadic senders.
//
// The `va_list` half stays in `Quake/host_glue.c`, which formats into its own
// `char string[1024]` and hands the finished text down. Each core below takes
// that string and performs only the message writes.

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_host_sv_client_printf(string: *const c_char) -> Raise {
    // SAFETY: `string` is the glue's stack buffer, live for the call.
    unsafe {
        let hc = host_client_get();
        write_batch(
            ptr::addr_of_mut!((*hc).message).cast(),
            &[op_byte(SVC_PRINT), op_string(string)],
        )
    }
}

/// The Rust core of `SV_BroadcastPrintf`. Split out so in-file callers
/// (`Host_Callback_Notify`) can reach it without going through the C wrapper,
/// which would `Host_Reraise` across this Rust frame.
unsafe fn sv_broadcast_printf_core(string: *const c_char) -> Raise {
    // SAFETY: `string` is live for the call; `svs.clients` is the engine array.
    unsafe {
        let svsp = svs_p();
        let maxclients = ptr::addr_of!((*svsp).maxclients).read();
        let clients = ptr::addr_of!((*svsp).clients).read();
        for i in 0..maxclients {
            let client = clients.offset(i as isize);
            if ptr::addr_of!((*client).active).read() && ptr::addr_of!((*client).spawned).read() {
                raise!(write_batch(
                    ptr::addr_of_mut!((*client).message).cast(),
                    &[op_byte(SVC_PRINT), op_string(string)],
                ));
            }
        }
    }
    g::HOST_GUARD_OK
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_host_sv_broadcast_printf(string: *const c_char) -> Raise {
    // SAFETY: see `sv_broadcast_printf_core`.
    unsafe { sv_broadcast_printf_core(string) }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_host_client_commands(string: *const c_char) -> Raise {
    // SAFETY: `string` is the glue's stack buffer, live for the call.
    unsafe {
        let hc = host_client_get();
        write_batch(
            ptr::addr_of_mut!((*hc).message).cast(),
            &[op_byte(SVC_STUFFTEXT), op_string(string)],
        )
    }
}

// ---------------------------------------------------------------------------
// host.c:590 -- SV_DropClient

/// The Rust core. `Host_ShutdownServer` calls this directly rather than the C
/// wrapper, for the same reason as `sv_broadcast_printf_core`.
unsafe fn sv_drop_client_core(crash: c::qboolean) -> Raise {
    // SAFETY: engine state; every pointer below is engine-owned.
    unsafe {
        let hc = host_client_get();

        if !crash {
            // send any final messages (don't check for errors)
            let sock = ptr::addr_of!((*hc).netconnection).read();
            let mut can = 0;
            raise!(g::Host_Glue_NetCanSendMessage(sock.cast(), &mut can));
            if can != 0 {
                raise!(write_batch(
                    ptr::addr_of_mut!((*hc).message).cast(),
                    &[op_byte(SVC_DISCONNECT)],
                ));
                let mut sent = 0;
                raise!(g::Host_Glue_NetSendMessage(
                    sock.cast(),
                    ptr::addr_of_mut!((*hc).message).cast(),
                    &mut sent
                ));
            }

            if !ptr::addr_of!((*hc).edict).read().is_null() && ptr::addr_of!((*hc).spawned).read() {
                // call the prog function for removing a client
                // this will set the body to a dead frame, among other things
                let oldvm = ptr::addr_of_mut!(c::qcvm).read();
                gsv::PR_SwitchQCVM(ptr::null_mut());
                gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast());
                let gv = pgs();
                let save_self = ptr::addr_of!((*gv).self_).read();
                let mut prog = 0;
                raise!(g::Host_Glue_EdictToProg(
                    ptr::addr_of!((*hc).edict).read().cast(),
                    &mut prog
                ));
                ptr::addr_of_mut!((*gv).self_).write(prog);
                let func = ptr::addr_of!((*gv).ClientDisconnect).read();
                raise!(g::Host_Glue_PR_ExecuteProgram(func as c_int));
                ptr::addr_of_mut!((*gv).self_).write(save_self);
                gsv::PR_SwitchQCVM(ptr::null_mut());
                gsv::PR_SwitchQCVM(oldvm.cast());
            }

            c::Sys_Printf(
                c"Client %s removed\n".as_ptr(),
                ptr::addr_of!((*hc).name).cast::<c_char>(),
            );
        }

        // break the net connection
        raise!(g::Host_Glue_NET_Close(
            ptr::addr_of!((*hc).netconnection).read().cast()
        ));
        ptr::addr_of_mut!((*hc).netconnection).write(ptr::null_mut());

        // release any delta state
        raise!(g::Host_Glue_SVFTE_DestroyFrames(hc.cast()));

        // free the client (the body stays around)
        ptr::addr_of_mut!((*hc).active).write(false);
        ptr::addr_of_mut!((*hc).name).cast::<c_char>().write(0);
        ptr::addr_of_mut!((*hc).old_frags).write(-999999);
        let na = ptr::addr_of_mut!(c::net_activeconnections);
        na.write(na.read() - 1);

        // send notification to all clients
        let svsp = svs_p();
        let maxclients = ptr::addr_of!((*svsp).maxclients).read();
        let base = ptr::addr_of!((*svsp).clients).read();
        let slot = hc.offset_from(base) as c_int;
        for i in 0..maxclients {
            let client = base.offset(i as isize);
            if !ptr::addr_of!((*client).knowntoqc).read() {
                continue;
            }

            raise!(write_batch(
                ptr::addr_of_mut!((*client).message).cast(),
                &[
                    op_byte(SVC_UPDATENAME),
                    op_byte(slot),
                    op_string(c"".as_ptr()),
                    op_byte(SVC_UPDATECOLORS),
                    op_byte(slot),
                    op_byte(0),
                    op_byte(SVC_UPDATEFRAGS),
                    op_byte(slot),
                    op_short(0),
                ],
            ));
        }
    }
    g::HOST_GUARD_OK
}

#[no_mangle]
pub extern "C" fn quake_rs_host_sv_drop_client(crash: c::qboolean) -> Raise {
    // SAFETY: see `sv_drop_client_core`.
    unsafe { sv_drop_client_core(crash) }
}

// ---------------------------------------------------------------------------
// host.c:661 -- Host_ShutdownServer

#[no_mangle]
pub extern "C" fn quake_rs_host_shutdown_server(crash: c::qboolean) -> Raise {
    // SAFETY: engine state, single-threaded.
    unsafe {
        let svp = sv_p();
        if !ptr::addr_of!((*svp).active).read() {
            return g::HOST_GUARD_OK;
        }

        ptr::addr_of_mut!((*svp).active).write(false);

        // stop all client sounds immediately
        if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED {
            raise!(g::Host_Glue_CL_Disconnect());
        }

        // flush any pending messages - like the score!!!
        let svsp = svs_p();
        let start = c::Sys_DoubleTime();
        let mut count;
        loop {
            count = 0;
            let maxclients = ptr::addr_of!((*svsp).maxclients).read();
            let base = ptr::addr_of!((*svsp).clients).read();
            let mut i = 0;
            host_client_set(base);
            while i < maxclients {
                let hc = host_client_get();
                if ptr::addr_of!((*hc).active).read()
                    && ptr::addr_of!((*hc).message.cursize).read() != 0
                    && !ptr::addr_of!((*hc).netconnection).read().is_null()
                {
                    let sock = ptr::addr_of!((*hc).netconnection).read();
                    let mut can = 0;
                    raise!(g::Host_Glue_NetCanSendMessage(sock.cast(), &mut can));
                    if can != 0 {
                        let mut sent = 0;
                        raise!(g::Host_Glue_NetSendMessage(
                            sock.cast(),
                            ptr::addr_of_mut!((*hc).message).cast(),
                            &mut sent
                        ));
                        sz_clear(ptr::addr_of_mut!((*hc).message));
                    } else {
                        let mut got = 0;
                        raise!(g::Host_Glue_NetGetMessage(sock.cast(), &mut got));
                        count += 1;
                    }
                }
                i += 1;
                host_client_set(host_client_get().offset(1));
            }
            if (c::Sys_DoubleTime() - start) > 3.0 {
                break;
            }
            if count == 0 {
                break;
            }
        }

        // make sure all the clients know we're disconnecting
        let mut sent_count = 0;
        raise!(g::Host_Glue_BroadcastDisconnect(&mut sent_count));
        if sent_count != 0 {
            c::Con_Printf(
                c"Host_ShutdownServer: NET_SendToAll failed for %u clients\n".as_ptr(),
                sent_count as c_uint,
            );
        }

        gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*svp).qcvm).cast());
        {
            let maxclients = ptr::addr_of!((*svsp).maxclients).read();
            let base = ptr::addr_of!((*svsp).clients).read();
            let mut i = 0;
            host_client_set(base);
            while i < maxclients {
                if ptr::addr_of!((*host_client_get()).active).read() {
                    // The C wrapper re-raises; call the core and propagate.
                    raise!(sv_drop_client_core(crash));
                }
                i += 1;
                host_client_set(host_client_get().offset(1));
            }
        }

        ptr::addr_of_mut!((*vm()).worldmodel).write(ptr::null_mut());
        gsv::PR_SwitchQCVM(ptr::null_mut());

        //
        // clear structures
        //
        //	memset (&sv, 0, sizeof(sv)); // ServerSpawn already do this by Host_ClearMemory
        let bytes =
            ptr::addr_of!((*svsp).maxclientslimit).read() as usize * core::mem::size_of::<Client>();
        ptr::write_bytes(ptr::addr_of!((*svsp).clients).read().cast::<u8>(), 0, bytes);
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:735 -- Host_ClearMemory

#[no_mangle]
pub extern "C" fn quake_rs_host_clear_memory() -> Raise {
    // SAFETY: engine state, single-threaded.
    unsafe {
        let clp = cl_p();
        let svp = sv_p();

        let cl_vm = ptr::addr_of_mut!((*clp).qcvm);
        if ptr::addr_of!((*cl_vm).extfuncs.csqc_shutdown).read() != 0 {
            gsv::PR_SwitchQCVM(cl_vm.cast());
            let cur = vm();
            let func = ptr::addr_of!((*cur).extfuncs.csqc_shutdown).read();
            raise!(g::Host_Glue_PR_ExecuteProgram(func as c_int));
            ptr::addr_of_mut!((*cur).extfuncs.csqc_shutdown).write(0);
            gsv::PR_SwitchQCVM(ptr::null_mut());
        }

        c::Con_DPrintf(c"Clearing memory\n".as_ptr());
        raise!(g::Host_Glue_Mod_ClearAll());
        raise!(g::Host_Glue_Sky_ClearAll());
        if !ptr::addr_of!(c::isDedicated).read() {
            raise!(g::Host_Glue_S_ClearAll());
        }
        ptr::addr_of_mut!((*cls_p()).signon).write(0);
        raise!(g::Host_Glue_PR_ClearProgs(
            ptr::addr_of_mut!((*svp).qcvm).cast()
        ));
        // spike -- this is dynamic too, now
        c::Mem_Free(ptr::addr_of!((*svp).static_entities).read().cast());
        for i in 1..MAX_PARTICLETYPES {
            c::Mem_Free(
                ptr::addr_of!((*svp).particle_precache)
                    .cast::<*const c_char>()
                    .add(i)
                    .read()
                    .cast(),
            );
        }
        ptr::write_bytes(svp.cast::<u8>(), 0, core::mem::size_of::<Server>());

        raise!(g::Host_Glue_CL_FreeState());
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:773 -- Host_FilterTime
//
// C computes `delta_since_last_frame` and `maxfps` in `double` and narrows
// each on assignment to its `float` local; the casts below reproduce both
// narrowings, and the `float` comparison that follows, exactly (ADR-010).

#[no_mangle]
pub extern "C" fn quake_rs_host_filter_time(time: c_float) -> c::qboolean {
    // SAFETY: engine timing globals, single-threaded.
    unsafe {
        let realtime = ptr::addr_of_mut!(gsv::realtime);
        realtime.write(realtime.read() + time as f64);
        let delta_since_last_frame =
            (realtime.read() - ptr::addr_of!(g::oldrealtime).read()) as c_float;

        if cvar_value(ptr::addr_of!(g::host_maxfps)) != 0.0 {
            // johnfitz -- max fps cvar
            let maxfps = clamp_f64(
                10.0,
                cvar_value(ptr::addr_of!(g::host_maxfps)) as f64,
                1000.0,
            ) as c_float;

            // Check if we still have more than 2ms till next frame and if so wait for "1ms"
            // E.g. Windows is not a real time OS and the sleeps can vary in length even with timeBeginPeriod(1)
            let min_frame_time = 1.0f32 / maxfps;
            if (min_frame_time - delta_since_last_frame) > (2.0f32 / 1000.0f32) {
                g::SDL_Delay(1);
            }

            if !ptr::addr_of!((*cls_p()).timedemo).read()
                && (delta_since_last_frame < min_frame_time)
            {
                return false; // framerate is too high
                              // johnfitz
            }
        }

        ptr::addr_of_mut!(g::host_rawframetime).write(delta_since_last_frame as f64);
        let host_frametime = ptr::addr_of_mut!(c::host_frametime);
        host_frametime.write(delta_since_last_frame as f64);
        ptr::addr_of_mut!(g::oldrealtime).write(realtime.read());

        let demospeed = ptr::addr_of!((*cls_p()).demospeed).read();
        if ptr::addr_of!((*cls_p()).demoplayback).read() && demospeed != 1.0 && demospeed > 0.0 {
            host_frametime.write(host_frametime.read() * demospeed as f64);
        }
        // johnfitz -- host_timescale is more intuitive than host_framerate
        else if cvar_value(ptr::addr_of!(g::host_timescale)) > 0.0 {
            host_frametime
                .write(host_frametime.read() * cvar_value(ptr::addr_of!(g::host_timescale)) as f64);
        }
        // johnfitz
        else if cvar_value(ptr::addr_of!(g::host_framerate)) > 0.0 {
            host_frametime.write(cvar_value(ptr::addr_of!(g::host_framerate)) as f64);
        }
        // don't allow really long or short frames
        else if cvar_value(ptr::addr_of!(g::host_maxfps)) != 0.0 {
            // johnfitz -- use CLAMP
            host_frametime.write(clamp_f64(0.0001, host_frametime.read(), 0.1));
        }
    }
    true
}

// ---------------------------------------------------------------------------
// host.c:822 -- Host_GetConsoleCommands

#[no_mangle]
pub extern "C" fn quake_rs_host_get_console_commands() -> Raise {
    // SAFETY: `Sys_ConsoleInput` returns an engine-owned buffer or NULL.
    unsafe {
        if !ptr::addr_of!(c::isDedicated).read() {
            return g::HOST_GUARD_OK; // no stdin necessary in graphical mode
        }

        loop {
            let cmd = g::Sys_ConsoleInput();
            if cmd.is_null() {
                break;
            }
            raise!(g::Host_Glue_CbufAddText(cmd));
        }
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:843 -- Host_ServerFrame

#[no_mangle]
pub extern "C" fn quake_rs_host_server_frame() -> Raise {
    // SAFETY: engine state, single-threaded.
    unsafe {
        let (mut t0, mut t1, mut t2, mut t3);
        t0 = 0.0f64;
        t1 = 0.0f64;
        t2 = 0.0f64;
        t3 = 0.0f64;

        let sv_speeds_on = cvar_value(ptr::addr_of!(gsp::sv_speeds)) != 0.0;
        if sv_speeds_on {
            t0 = c::Sys_DoubleTime();
        }

        // run the world state
        ptr::addr_of_mut!((*pgs()).frametime)
            .write(ptr::addr_of!(c::host_frametime).read() as c_float);

        // set the time and clear the general datagram
        raise!(g::Host_Glue_SV_ClearDatagram());

        // check for new clients
        raise!(g::Host_Glue_SV_CheckForNewClients());

        // read client messages
        raise!(g::Host_Glue_SV_RunClients());

        if sv_speeds_on {
            t1 = c::Sys_DoubleTime();
        }

        // move things around and think
        // always pause in single player if in console or menus
        if !ptr::addr_of!((*sv_p()).paused).read()
            && (ptr::addr_of!((*svs_p()).maxclients).read() > 1
                || ptr::addr_of!(gsu::key_dest).read() == KEY_GAME)
        {
            raise!(g::Host_Glue_SV_Physics());
        }

        if sv_speeds_on {
            t2 = c::Sys_DoubleTime();
        }

        // johnfitz -- devstats
        // the count is only observable through the devstats overlay and a developer warning, so don't walk every edict per tick unless one of them can see it
        if ptr::addr_of!((*cls_p()).signon).read() == SIGNONS
            && (cvar_value(ptr::addr_of!(g::devstats)) != 0.0
                || cvar_value(ptr::addr_of!(c::developer)) != 0.0)
        {
            let mut active = 0;
            let num_edicts = ptr::addr_of!((*vm()).num_edicts).read();
            for i in 0..num_edicts {
                let mut ent: *mut c_void = ptr::null_mut();
                raise!(g::Host_Glue_EdictNum(i, &mut ent));
                if !ptr::addr_of!((*ent.cast::<Edict>()).free).read() {
                    active += 1;
                }
            }
            if active > 600 && ptr::addr_of!(g::dev_peakstats.edicts).read() <= 600 {
                c::Con_DWarning(
                    c"%i edicts exceeds standard limit of 600 (max = %d).\n".as_ptr(),
                    active,
                    ptr::addr_of!((*vm()).max_edicts).read(),
                );
            }
            ptr::addr_of_mut!(gcp::dev_stats.edicts).write(active);
            let peak = ptr::addr_of_mut!(g::dev_peakstats.edicts);
            peak.write(if active > peak.read() {
                active
            } else {
                peak.read()
            });
        }
        // johnfitz

        if sv_speeds_on {
            t3 = c::Sys_DoubleTime();
        }

        // send all messages to the clients
        raise!(g::Host_Glue_SV_SendClientMessages());

        if !sv_speeds_on && SVF_INTERVAL_START != 0.0 {
            // reset on toggle so a later enable doesn't average across the gap
            SVF_CLIENTS_MS = 0.0;
            SVF_PHYSICS_MS = 0.0;
            SVF_STATS_MS = 0.0;
            SVF_SEND_MS = 0.0;
            sv_speeds_counters_reset();
            SVF_TICKS = 0;
            SVF_INTERVAL_START = 0.0;
        }

        if sv_speeds_on {
            let t4 = c::Sys_DoubleTime();
            SVF_CLIENTS_MS += (t1 - t0) * 1000.0;
            SVF_PHYSICS_MS += (t2 - t1) * 1000.0;
            SVF_STATS_MS += (t3 - t2) * 1000.0;
            SVF_SEND_MS += (t4 - t3) * 1000.0;
            SVF_TICKS += 1;
            if SVF_INTERVAL_START == 0.0 {
                SVF_INTERVAL_START = t0;
            }
            if t4 - SVF_INTERVAL_START >= 1.0 {
                let ticks = SVF_TICKS as f64;
                let physics = SVF_PHYSICS_MS / ticks;
                let pushers = ptr::addr_of!(g::sv_speeds_pusher_ms).read() / ticks;
                let thinks = ptr::addr_of!(g::sv_speeds_think_ms).read() / ticks;
                let build = ptr::addr_of!(g::sv_speeds_build_ms).read() / ticks;
                c::Con_Printf(
                    c"sv_speeds: %3d ticks | clients %.3f | physics %.3f [pushers %.3f (%.0f) thinks %.3f (%.0f) build %.3f loop %.3f] | stats %.3f | send %.3f ms/tick | %d edicts, %.0f pushables, %.0f grid entries\n"
                        .as_ptr(),
                    SVF_TICKS,
                    SVF_CLIENTS_MS / ticks,
                    physics,
                    pushers,
                    ptr::addr_of!(g::sv_speeds_pushers).read() as f64 / ticks,
                    thinks,
                    ptr::addr_of!(g::sv_speeds_thinks).read() as f64 / ticks,
                    build,
                    physics - pushers - thinks - build,
                    SVF_STATS_MS / ticks,
                    SVF_SEND_MS / ticks,
                    ptr::addr_of!((*vm()).num_edicts).read(),
                    ptr::addr_of!(g::sv_speeds_pushables).read() as f64 / ticks,
                    ptr::addr_of!(g::sv_speeds_grid_entries).read() as f64 / ticks,
                );
                SVF_CLIENTS_MS = 0.0;
                SVF_PHYSICS_MS = 0.0;
                SVF_STATS_MS = 0.0;
                SVF_SEND_MS = 0.0;
                sv_speeds_counters_reset();
                SVF_TICKS = 0;
                SVF_INTERVAL_START = t4;
            }
        }
    }
    g::HOST_GUARD_OK
}

/// `host.c:903-904` / `:930-931` -- the seven `sv_phys.c` counters
/// `Host_ServerFrame` zeroes in both reset paths.
#[inline]
unsafe fn sv_speeds_counters_reset() {
    // SAFETY: plain C globals with external linkage.
    unsafe {
        ptr::addr_of_mut!(g::sv_speeds_think_ms).write(0.0);
        ptr::addr_of_mut!(g::sv_speeds_pusher_ms).write(0.0);
        ptr::addr_of_mut!(g::sv_speeds_build_ms).write(0.0);
        ptr::addr_of_mut!(g::sv_speeds_thinks).write(0);
        ptr::addr_of_mut!(g::sv_speeds_pushers).write(0);
        ptr::addr_of_mut!(g::sv_speeds_pushables).write(0);
        ptr::addr_of_mut!(g::sv_speeds_grid_entries).write(0);
    }
}

// ---------------------------------------------------------------------------
// host.c:944 -- CL_LoadCSProgs (static in C, private here).

/// `progs.h` `qcvm->globals`, the base of the `G_*` macro block.
#[inline]
unsafe fn globals(vm: *mut QcVm) -> *mut c_float {
    // SAFETY: `vm` is the engine's live qcvm.
    unsafe { ptr::addr_of!((*vm).globals).read() }
}

unsafe fn cl_load_csprogs() -> Raise {
    // SAFETY: engine state, single-threaded; the ambient qcvm is selected and
    // cleared here in exactly the C order (ADR-008).
    unsafe {
        let clp = cl_p();
        raise!(g::Host_Glue_PR_ClearProgs(
            ptr::addr_of_mut!((*clp).qcvm).cast()
        ));

        // only try to use csqc if qc extensions are enabled.
        if cvar_value(ptr::addr_of!(c::world::pr_checkextension)) == 0.0
            || cvar_value(ptr::addr_of!(g::cl_nocsqc)) != 0.0
        {
            return g::HOST_GUARD_OK;
        }

        let mut versionedname = [0 as c_char; MAX_QPATH];
        gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*clp).qcvm).cast());

        let csqchash = g::strtoul(
            gcl::Info_GetKey(
                ptr::addr_of!((*clp).serverinfo).cast::<c_char>(),
                c"*csprogs".as_ptr(),
                versionedname.as_mut_ptr(),
                MAX_QPATH,
            ),
            ptr::null_mut(),
            0,
        ) as c_uint;

        {
            let mut b = CBuf::new(versionedname.as_mut_ptr(), MAX_QPATH);
            b.lit(b"csprogsvers/");
            b.hex(csqchash);
            b.lit(b".dat");
        }

        // try csprogs.dat first, then fall back on progs.dat in case someone tried merging the two.
        // we only care about it if it actually contains a CSQC_DrawHud, otherwise its either just a (misnamed) ssqc progs or a full csqc progs that would just
        // crash us on 3d stuff.
        let candidates: [*const c_char; 3] = [
            versionedname.as_ptr(),
            c"csprogs.dat".as_ptr(),
            c"progs.dat".as_ptr(),
        ];
        let mut loaded = false;
        for name in candidates {
            let mut ok: c_int = 0;
            raise!(g::Host_Glue_PRLoadProgs(
                name,
                PROGHEADER_CRC,
                ptr::addr_of!(g::pr_csqcbuiltins).cast(),
                ptr::addr_of!(g::pr_csqcnumbuiltins).read() as usize,
                &mut ok,
            ));
            if ok != 0 && ptr::addr_of!((*vm()).extfuncs.csqc_draw_hud).read() != 0 {
                loaded = true;
                break;
            }
        }

        if !loaded {
            raise!(g::Host_Glue_PR_ClearProgs(vm().cast()));
            gsv::PR_SwitchQCVM(ptr::null_mut());
            return g::HOST_GUARD_OK;
        }

        let cur = vm();
        ptr::addr_of_mut!((*cur).max_edicts).write(clamp_i32(
            MIN_EDICTS,
            cvar_value(ptr::addr_of!(gsv::max_edicts)) as c_int,
            MAX_EDICTS,
        ));
        let bytes =
            ptr::addr_of!((*cur).max_edicts).read() * ptr::addr_of!((*cur).edict_size).read();
        ptr::addr_of_mut!((*cur).edicts).write(c::Mem_Alloc(bytes as usize).cast::<Edict>());
        ptr::addr_of_mut!((*cur).reserved_edicts).write(1);
        ptr::addr_of_mut!((*cur).num_edicts).write(1);

        // set debug fiels for all max_edicts
        #[cfg(feature = "engine-debug")]
        {
            let base = ptr::addr_of!((*cur).edicts).read().cast::<u8>();
            let stride = ptr::addr_of!((*cur).edict_size).read() as usize;
            for i in 0..ptr::addr_of!((*cur).max_edicts).read() {
                let e = base.add(stride * i as usize).cast::<Edict>();
                ptr::addr_of_mut!((*e).qcvm_owner).write(cur);
                ptr::addr_of_mut!((*e).edict_ptr).write(e);
                ptr::addr_of_mut!((*e).edict_num).write(i as u64);
            }
        }

        // no simplecsqc entry points... abort entirely!
        if ptr::addr_of!((*cur).extfuncs.csqc_draw_hud).read() == 0 {
            raise!(g::Host_Glue_PR_ClearProgs(cur.cast()));
            gsv::PR_SwitchQCVM(ptr::null_mut());
            return g::HOST_GUARD_OK;
        }

        // set a few globals, if they exist
        let maxclients_g = ptr::addr_of!((*cur).extglobals.maxclients).read();
        if !maxclients_g.is_null() {
            maxclients_g.write(ptr::addr_of!((*clp).maxclients).read() as c_float);
        }
        let gv = pgs();
        ptr::addr_of_mut!((*gv).time).write(ptr::addr_of!((*clp).time).read() as c_float);
        let mut s: c_int = 0;
        raise!(g::Host_Glue_PRSetEngineString(
            ptr::addr_of!((*clp).mapname).cast::<c_char>(),
            &mut s
        ));
        ptr::addr_of_mut!((*gv).mapname).write(s);
        ptr::addr_of_mut!((*gv).total_monsters).write(
            ptr::addr_of!((*clp).statsf)
                .cast::<f32>()
                .add(STAT_TOTALMONSTERS)
                .read(),
        );
        ptr::addr_of_mut!((*gv).total_secrets).write(
            ptr::addr_of!((*clp).statsf)
                .cast::<f32>()
                .add(STAT_TOTALSECRETS)
                .read(),
        );
        let gametype = ptr::addr_of!((*clp).gametype).read();
        let maxclients = ptr::addr_of!((*clp).maxclients).read();
        ptr::addr_of_mut!((*gv).deathmatch).write(gametype as c_float);
        ptr::addr_of_mut!((*gv).coop)
            .write(((gametype == GAME_COOP) && maxclients != 1) as c_int as c_float);
        // this is a guess, but is important for scoreboards.
        let localnum_g = ptr::addr_of!((*cur).extglobals.player_localnum).read();
        if !localnum_g.is_null() {
            localnum_g.write((ptr::addr_of!((*clp).viewentity).read() - 1) as c_float);
        }

        // set a few worldspawn fields too
        let world = ptr::addr_of!((*cur).edicts).read();
        let worldmodel = ptr::addr_of!((*clp).worldmodel).read();
        ptr::addr_of_mut!((*world).v.solid).write(SOLID_BSP);
        ptr::addr_of_mut!((*world).v.modelindex).write(1.0);
        let mut model_s: c_int = 0;
        raise!(g::Host_Glue_PRSetEngineString(
            ptr::addr_of!((*worldmodel).name).cast::<c_char>(),
            &mut model_s
        ));
        ptr::addr_of_mut!((*world).v.model).write(model_s);
        for i in 0..3usize {
            ptr::addr_of_mut!((*world).v.mins)
                .cast::<f32>()
                .add(i)
                .write(
                    ptr::addr_of!((*worldmodel).mins)
                        .cast::<f32>()
                        .add(i)
                        .read(),
                );
            ptr::addr_of_mut!((*world).v.maxs)
                .cast::<f32>()
                .add(i)
                .write(
                    ptr::addr_of!((*worldmodel).maxs)
                        .cast::<f32>()
                        .add(i)
                        .read(),
                );
        }
        let mut msg_s: c_int = 0;
        raise!(g::Host_Glue_PRSetEngineString(
            ptr::addr_of!((*clp).levelname).cast::<c_char>(),
            &mut msg_s
        ));
        ptr::addr_of_mut!((*world).v.message).write(msg_s);

        // and call the init function... if it exists.
        ptr::addr_of_mut!((*cur).worldmodel).write(worldmodel.cast());
        raise!(g::Host_Glue_SV_ClearWorld());
        let init = ptr::addr_of!((*cur).extfuncs.csqc_init).read();
        if init != 0 {
            let maj = VKQUAKE_VERSION as c_int;
            let min = ((VKQUAKE_VERSION - maj as f64) * 100.0) as c_int;
            let gp = globals(cur);
            gp.add(OFS_PARM0).write(0.0);
            let mut engine_s: c_int = 0;
            raise!(g::Host_Glue_PRSetEngineString(
                c"vkQuake".as_ptr(),
                &mut engine_s
            ));
            gp.add(OFS_PARM1).cast::<c_int>().write(engine_s);
            gp.add(OFS_PARM2)
                .write((10000 * maj + 100 * min + VKQUAKE_VER_PATCH) as c_float);
            raise!(g::Host_Glue_PR_ExecuteProgram(init as c_int));
        }

        gsv::PR_SwitchQCVM(ptr::null_mut());
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:1030 -- Host_UpdateSteamStatus
//
// Updates the Steam rich presence status when the map or
// player counts change (based on the Ironwail equivalent)

unsafe fn host_update_steam_status() {
    // SAFETY: engine state, single-threaded; `Steam_SetStatus_*` are plain.
    unsafe {
        let clp = cl_p();
        if ptr::addr_of!(gsv::realtime).read() < STEAM_NEXTUPDATE {
            return;
        }
        STEAM_NEXTUPDATE = ptr::addr_of!(gsv::realtime).read() + 0.25;

        let mut mapname = [0 as c_char; 128];
        let mut players: c_int = 0;
        let mut maxplayers: c_int = 0;

        if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED
            && !ptr::addr_of!((*clp).worldmodel).read().is_null()
        {
            // strip Quake color codes and control characters from the level name
            let src = ptr::addr_of!((*clp).levelname).cast::<c_char>();
            let mut i = 0usize;
            let mut len = 0usize;
            while *src.add(i) != 0 && len < mapname.len() - 1 {
                let ch = *src.add(i) & 0x7f;
                i += 1;
                if ch >= 32 {
                    mapname[len] = ch;
                    len += 1;
                }
            }
            mapname[len] = 0;
            if mapname[0] == 0 {
                let worldmodel = ptr::addr_of!((*clp).worldmodel).read();
                c::COM_StripExtension(
                    c::COM_SkipPath(ptr::addr_of!((*worldmodel).name).cast::<c_char>()),
                    mapname.as_mut_ptr(),
                    mapname.len(),
                );
            }

            maxplayers = ptr::addr_of!((*clp).maxclients).read();
            let scores = ptr::addr_of!((*clp).scores).read();
            for i in 0..maxplayers {
                if ptr::addr_of!((*scores.offset(i as isize)).name)
                    .cast::<c_char>()
                    .read()
                    != 0
                {
                    players += 1;
                }
            }
        }

        if c_str_eq(
            mapname.as_ptr(),
            ptr::addr_of!(STEAM_LASTMAP).cast::<c_char>(),
        ) && players == STEAM_LASTPLAYERS
            && maxplayers == STEAM_LASTMAXPLAYERS
        {
            return;
        }
        q_strlcpy(
            ptr::addr_of_mut!(STEAM_LASTMAP).cast::<c_char>(),
            mapname.as_ptr(),
            128,
        );
        STEAM_LASTPLAYERS = players;
        STEAM_LASTMAXPLAYERS = maxplayers;

        if mapname[0] == 0 {
            g::Steam_SetStatus_Menu();
        } else if maxplayers > 1 {
            g::Steam_SetStatus_Multiplayer(players, maxplayers, mapname.as_ptr());
        } else {
            g::Steam_SetStatus_SinglePlayer(mapname.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// host.c:1085 -- _Host_Frame, minus its setjmp (which stays in
// `Host_Glue_FrameInner`). Runs all active servers.

#[no_mangle]
pub extern "C" fn quake_rs_host_frame_core(time: f64) -> Raise {
    // SAFETY: engine state, single-threaded.
    unsafe {
        // keep the random time dependent
        c::COM_Rand();

        // decide the simulation time
        // for renderer/server isolation
        FRAME_ACCUMTIME += if ptr::addr_of!(gcl::host_netinterval).read() != 0.0 {
            clamp_f64(0.0, time, 0.2)
        } else {
            0.0
        };
        if !quake_rs_host_filter_time(time as c_float) {
            return g::HOST_GUARD_OK; // don't run too fast, or packets will flood out
        }

        let speeds = cvar_value(ptr::addr_of!(g::host_speeds)) != 0.0;
        if speeds {
            FRAME_TIME3 = c::Sys_DoubleTime();
        }

        if !ptr::addr_of!(g::no_rendering).read() {
            // get new key events
            raise!(g::Host_Glue_Key_UpdateForDest());
            raise!(g::Host_Glue_IN_UpdateInputMode());
            g::Sys_SendKeyEvents();

            // allow mice or other external controllers to add commands
            raise!(g::Host_Glue_IN_Commands());

            // handle mouse interaction with the console (selection, links)
            raise!(g::Host_Glue_Con_UpdateMouseState());
        }

        // check the stdin for commands (dedicated servers)
        raise!(quake_rs_host_get_console_commands());

        // process console commands
        raise!(g::Host_Glue_Cbuf_Execute());

        raise!(g::Host_Glue_NET_Poll());

        let clp = cl_p();
        if ptr::addr_of!((*clp).sendprespawn).read() {
            raise!(cl_load_csprogs());

            ptr::addr_of_mut!((*clp).sendprespawn).write(false);
            raise!(write_batch(
                ptr::addr_of_mut!((*cls_p()).message).cast(),
                &[op_byte(CLC_STRINGCMD), op_string(c"prespawn".as_ptr())],
            ));
            ptr::addr_of_mut!(gcp::vid.recalc_refdef).write(1);
        }

        raise!(g::Host_Glue_CL_AccumulateCmd());
        raise!(g::Host_Glue_M_UpdateMouse());

        // Run the server+networking (client->server->client), at a different rate from everyt
        let host_frametime = ptr::addr_of_mut!(c::host_frametime);
        loop {
            let netinterval = ptr::addr_of!(gcl::host_netinterval).read();
            if !(netinterval == 0.0 || FRAME_ACCUMTIME >= netinterval as f64) {
                break;
            }

            let realframetime = host_frametime.read();
            if netinterval != 0.0 && !ptr::addr_of!(c::isDedicated).read() {
                if ptr::addr_of!((*sv_p()).active).read() {
                    if ptr::addr_of!(c::listening).read() {
                        host_frametime.write(if FRAME_ACCUMTIME < 0.017 {
                            FRAME_ACCUMTIME
                        } else {
                            0.017
                        });
                    } else {
                        host_frametime.write(netinterval as f64);
                    }
                } else {
                    host_frametime.write(FRAME_ACCUMTIME);
                }

                FRAME_ACCUMTIME -= host_frametime.read();
                let timescale = cvar_value(ptr::addr_of!(g::host_timescale));
                if timescale > 0.0 {
                    host_frametime.write(host_frametime.read() * timescale as f64);
                } else if cvar_value(ptr::addr_of!(g::host_framerate)) != 0.0 {
                    host_frametime.write(cvar_value(ptr::addr_of!(g::host_framerate)) as f64);
                }
            }

            raise!(g::Host_Glue_CL_SendCmd());
            if ptr::addr_of!((*sv_p()).active).read() {
                gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*sv_p()).qcvm).cast());
                let r = quake_rs_host_server_frame();
                if r != 0 {
                    return r;
                }
                gsv::PR_SwitchQCVM(ptr::null_mut());
            }
            host_frametime.write(realframetime);
            g::Cbuf_Waited();

            if netinterval == 0.0 || ptr::addr_of!(c::isDedicated).read() {
                break;
            }
        }

        if !ptr::addr_of!((*clp).qcvm.progs).read().is_null() {
            gsv::PR_SwitchQCVM(ptr::addr_of_mut!((*clp).qcvm).cast());
            ptr::addr_of_mut!((*pgs()).frametime).write(host_frametime.read() as c_float);
            raise!(g::Host_Glue_SV_Physics());
            gsv::PR_SwitchQCVM(ptr::null_mut());
        }

        // fetch results from server
        if ptr::addr_of!((*cls_p()).state).read() == CA_CONNECTED {
            raise!(g::Host_Glue_CL_ReadFromServer());
        }

        // update video
        if speeds {
            FRAME_TIME1 = c::Sys_DoubleTime();
        }

        raise!(g::Host_Glue_SCR_UpdateScreen(1));

        raise!(g::Host_Glue_CL_RunParticles()); // johnfitz -- seperated from rendering

        if speeds {
            FRAME_TIME2 = c::Sys_DoubleTime();
        }

        // update audio
        raise!(g::Host_Glue_BGM_Update()); // adds music raw samples and/or advances midi driver
        if ptr::addr_of!((*cls_p()).signon).read() == SIGNONS {
            raise!(g::Host_Glue_SUpdate(
                ptr::addr_of!(g::r_origin).cast(),
                ptr::addr_of!(gcl::vpn).cast(),
                ptr::addr_of!(g::vright).cast(),
                ptr::addr_of!(g::vup).cast(),
            ));
            raise!(g::Host_Glue_CL_DecayLights());
        } else if !ptr::addr_of!(c::isDedicated).read() {
            let zero = ptr::addr_of!(g::vec3_origin).cast::<c_float>();
            raise!(g::Host_Glue_SUpdate(zero, zero, zero, zero));
        }

        raise!(g::Host_Glue_CDAudio_Update());

        host_update_steam_status();

        if speeds {
            let pass1 = (FRAME_TIME1 - FRAME_TIME3) * 1000.0;
            FRAME_TIME3 = c::Sys_DoubleTime();
            let pass2 = (FRAME_TIME2 - FRAME_TIME1) * 1000.0;
            let pass3 = (FRAME_TIME3 - FRAME_TIME2) * 1000.0;
            c::Con_Printf(
                c"%5.2f tot %5.2f server %5.2f gfx %5.2f snd\n".as_ptr(),
                pass1 + pass2 + pass3,
                pass1,
                pass2,
                pass3,
            );
        }

        raise!(g::Host_Glue_Harness_Frame());

        let fc = ptr::addr_of_mut!(c::host_framecount);
        fc.write(fc.read() + 1);
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:1236 -- Host_Frame

#[no_mangle]
pub extern "C" fn quake_rs_host_frame(time: f64) -> Raise {
    // SAFETY: engine state, single-threaded.
    unsafe {
        if cvar_value(ptr::addr_of!(g::serverprofile)) == 0.0 {
            g::Host_Glue_FrameInner(time);
            return g::HOST_GUARD_OK;
        }

        let time1 = c::Sys_DoubleTime();
        g::Host_Glue_FrameInner(time);
        let time2 = c::Sys_DoubleTime();

        PROFILE_TIMETOTAL += time2 - time1;
        PROFILE_TIMECOUNT += 1;

        if PROFILE_TIMECOUNT < 1000 {
            return g::HOST_GUARD_OK;
        }

        let m = (PROFILE_TIMETOTAL * 1000.0 / PROFILE_TIMECOUNT as f64) as c_int;
        PROFILE_TIMECOUNT = 0;
        PROFILE_TIMETOTAL = 0.0;
        let mut count: c_int = 0;
        let svsp = svs_p();
        let maxclients = ptr::addr_of!((*svsp).maxclients).read();
        let base = ptr::addr_of!((*svsp).clients).read();
        for i in 0..maxclients {
            if ptr::addr_of!((*base.offset(i as isize)).active).read() {
                count += 1;
            }
        }

        c::Con_Printf(c"serverprofile: %2i clients %2i msec\n".as_ptr(), count, m);
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:1277 -- Tests_Init
//
// The three commands only exist under `_DEBUG`; the glue exports the table and
// a count that is zero in a release build, so the loop is unconditional here.

unsafe fn tests_init() {
    // SAFETY: the table is a static C array of `host_glue_num_tests` entries.
    unsafe {
        let n = ptr::addr_of!(g::host_glue_num_tests).read();
        let names = ptr::addr_of!(g::host_glue_test_names).cast::<*const c_char>();
        let funcs = ptr::addr_of!(g::host_glue_test_funcs).cast::<Option<unsafe extern "C" fn()>>();
        for i in 0..n as usize {
            add_command(names.add(i).read(), funcs.add(i).read());
        }
    }
}

// ---------------------------------------------------------------------------
// host.c:1291 -- Host_Init

#[no_mangle]
pub extern "C" fn quake_rs_host_init() -> Raise {
    // SAFETY: engine bring-up, single-threaded, before any worker exists.
    unsafe {
        let parms = ptr::addr_of!(g::host_parms).read();
        ptr::addr_of_mut!(c::com_argc).write(ptr::addr_of!((*parms).argc).read());
        ptr::addr_of_mut!(c::com_argv).write(ptr::addr_of!((*parms).argv).read());

        raise!(g::Host_Glue_Mem_Init());
        raise!(g::Host_Glue_Tasks_Init());
        raise!(g::Host_Glue_Cbuf_Init());
        raise!(g::Host_Glue_Cmd_Init());
        raise!(g::Host_Glue_LOG_Init(parms.cast()));
        raise!(g::Host_Glue_Cvar_Init()); // johnfitz
        raise!(g::Host_Glue_COM_Init());
        raise!(g::Host_Glue_COM_InitFilesystem());
        raise!(quake_rs_host_init_local());
        raise!(g::Host_Glue_W_LoadWadFile()); // johnfitz -- filename is now hard-coded for honesty
        if ptr::addr_of!((*cls_p()).state).read() != CA_DEDICATED {
            raise!(g::Host_Glue_Key_Init());
            raise!(g::Host_Glue_Con_Init());
        }
        raise!(g::Host_Glue_PR_Init());
        raise!(g::Host_Glue_Mod_Init());
        raise!(g::Host_Glue_NET_Init());
        raise!(g::Host_Glue_SV_Init());

        c::Con_Printf(
            c"Exe: %s %s %s\n".as_ptr(),
            ptr::addr_of!(g::host_glue_build_time).read(),
            ptr::addr_of!(g::host_glue_build_date).read(),
            ptr::addr_of!(g::host_glue_build_suffix).read(),
        );

        // Rust migration: prove the quake_rs staticlib is linked and callable
        c::Con_DPrintf(
            c"quake_rs staticlib linked (ABI version %u)\n".as_ptr(),
            crate::QuakeRS_Version() as c_uint,
        );

        if ptr::addr_of!((*cls_p()).state).read() != CA_DEDICATED {
            let mut colormap: *mut c_void = ptr::null_mut();
            raise!(g::Host_Glue_ComLoadFile(
                c"gfx/colormap.lmp".as_ptr(),
                &mut colormap
            ));
            ptr::addr_of_mut!(g::host_colormap).write(colormap.cast::<u8>());
            if ptr::addr_of!(g::host_colormap).read().is_null() {
                c::Sys_Error(c"Couldn't load gfx/colormap.lmp".as_ptr());
            }

            raise!(g::Host_Glue_V_Init());
            raise!(g::Host_Glue_Chase_Init());
            raise!(g::Host_Glue_M_Init());
            raise!(g::Host_Glue_ExtraMaps_Init()); // johnfitz
            raise!(g::Host_Glue_M_CheckMods());
            raise!(g::Host_Glue_Modlist_Init()); // johnfitz
            raise!(g::Host_Glue_DemoList_Init()); // ericw
            raise!(g::Host_Glue_SaveList_Init());
            if !ptr::addr_of!(g::no_rendering).read() {
                raise!(g::Host_Glue_VID_Init());
                raise!(g::Host_Glue_IN_Init());
                raise!(g::Host_Glue_TexMgr_Init()); // johnfitz
                raise!(g::Host_Glue_Draw_Init());
                raise!(g::Host_Glue_SCR_Init());
                raise!(g::Host_Glue_R_Init());
                raise!(g::Host_Glue_S_Init());
                raise!(g::Host_Glue_CDAudio_Init());
                raise!(g::Host_Glue_BGM_Init());
                raise!(g::Host_Glue_Sbar_Init());
            } else {
                raise!(g::Host_Glue_R_InitParticles()); // particle simulation still runs headless
                if ptr::addr_of!(c::harness_sndhash).read() {
                    // -sndhash: the mixer must run headless, on the deterministic
                    // harness DMA clock (Phase 4 PCM-hash gate)
                    raise!(g::Host_Glue_S_Init());
                    raise!(g::Host_Glue_CDAudio_Init());
                    raise!(g::Host_Glue_BGM_Init());
                }
            }
            raise!(g::Host_Glue_CL_Init());
            tests_init();
        }

        raise!(g::Host_Glue_PScript_InitParticles());
        raise!(g::Host_Glue_LOC_Init()); // for 2021 rerelease support.

        raise!(g::Host_Glue_Harness_Init());
        raise!(g::Host_Glue_PR_TraceInit());

        ptr::addr_of_mut!(c::cvar_cmd::host_initialized).write(true);
        c::Con_Printf(c"\n========= Quake Initialized =========\n\n".as_ptr());

        // the folder from the selection dialog is only remembered now,
        // with the game data proven to actually work
        raise!(g::Host_Glue_COM_WriteSelectedBaseDir());

        if ptr::addr_of!((*cls_p()).state).read() != CA_DEDICATED {
            raise!(g::Host_Glue_CbufInsertText(c"exec quake.rc\n".as_ptr()));
            // johnfitz -- in case the vid mode was locked during vid_init, we can unlock it now.
            // note: two leading newlines because the command buffer swallows one of them.
            raise!(g::Host_Glue_CbufAddText(c"\n\nvid_unlock\n".as_ptr()));
        }

        if ptr::addr_of!((*cls_p()).state).read() == CA_DEDICATED {
            raise!(g::Host_Glue_CbufAddText(c"exec autoexec.cfg\n".as_ptr()));
            raise!(g::Host_Glue_CbufAddText(c"stuffcmds".as_ptr()));
            raise!(g::Host_Glue_Cbuf_Execute());
            if !ptr::addr_of!((*sv_p()).active).read() {
                raise!(g::Host_Glue_CbufAddText(c"map start\n".as_ptr()));
            }
        }
    }
    g::HOST_GUARD_OK
}

// ---------------------------------------------------------------------------
// host.c:1415 -- Host_Shutdown
//
// FIXME: this is a callback from Sys_Quit and Sys_Error.  It would be better
// to run quit through here before the final handoff to the sys code.

#[no_mangle]
pub extern "C" fn quake_rs_host_shutdown() -> Raise {
    // SAFETY: engine teardown, single-threaded.
    unsafe {
        // C's `assert` compiles out of a release build, so the check is gated
        // on the same flag `-D_DEBUG` sets (`engine-debug`).
        #[cfg(feature = "engine-debug")]
        assert!(!g::Tasks_IsWorker());

        if SHUTDOWN_ISDOWN {
            g::printf(c"recursive shutdown\n".as_ptr());
            return g::HOST_GUARD_OK;
        }
        SHUTDOWN_ISDOWN = true;

        raise!(g::Host_Glue_Harness_Shutdown());
        raise!(g::Host_Glue_PR_TraceShutdown());

        // keep Con_Printf from trying to update the screen
        ptr::addr_of_mut!(g::scr_disabled_for_loading).write(true);

        raise!(quake_rs_host_write_configuration());

        raise!(g::Host_Glue_NET_Shutdown());

        if ptr::addr_of!((*cls_p()).state).read() != CA_DEDICATED {
            if ptr::addr_of!(g::con_initialized).read() {
                raise!(g::Host_Glue_History_Shutdown());
            }
            raise!(g::Host_Glue_ExtraMaps_ShutDown());
            raise!(g::Host_Glue_BGM_Shutdown());
            raise!(g::Host_Glue_CDAudio_Shutdown());
            raise!(g::Host_Glue_S_Shutdown());
            raise!(g::Host_Glue_IN_Shutdown());
            raise!(g::Host_Glue_VID_Shutdown());
        }

        raise!(g::Host_Glue_Steam_Shutdown());

        raise!(g::Host_Glue_LOG_Close());

        raise!(g::Host_Glue_LOC_Shutdown());
    }
    g::HOST_GUARD_OK
}
