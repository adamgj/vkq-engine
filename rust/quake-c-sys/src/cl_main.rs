//! `Quake/cl_main_glue.c` declarations (Rust migration Phase 7 M7, T7.4).
//!
//! ADR-011: engine C symbols are declared only in this crate. `cl_main.c`
//! defined nineteen C-visible objects; ADR-007 moves exactly two of them --
//! `cl` and `cls` -- into Rust, and the other seventeen (the cvars,
//! `cl_lightstyle[]`, `cl_dlights[]`, the visedicts counters/arrays and
//! `needs_relink`) keep C storage in `Quake/cl_main_glue.c`, which is why they
//! are declared here rather than defined in `quake-capi`.
//!
//! `cl`/`cls`, `entity_t`, `refdef_t` and `entity_state_t` are mirror types,
//! so anything typed in terms of them lives in `quake-capi/src/cl_main.rs`;
//! this crate has no `[dependencies]`.
//!
//! `Host_Reraise` is deliberately absent: only `cl_main_glue.c` calls it
//! (ADR-009 rule 3). Each `ClMain_Glue_*` below returns a `Host_Guard` status
//! (0 = returned normally, 1 = `Host_Error`, 2 = `Sys_Error`) which the Rust
//! core propagates upward untouched.
//!
//! Five of the objects `cl_main.c` reads have external linkage but no header
//! declaration -- `host_netinterval` (`host.c:67`), `r_lerpmodels`,
//! `r_lerpmove`, `r_lerpturn` (`gl_rmisc.c`) and `vpn` (declared in both
//! `render.h:201` and `glquake.h:551`, neither of which is bindgen-clean).
//! `cl_main.c:69-71` declared the first four locally at file scope; the port
//! reaches them the same way, and their storage stays C-side.

use crate::cl_parse::{devstats_t, lightstyle_t};
use crate::cl_tent::dlight_t;
use crate::{cvar_t, qsocket_s, sizebuf_t};
use core::ffi::{c_char, c_float, c_int, c_ulong, c_void};

/// One buffered `MSG_Write*` op replayed by `ClMain_Glue_WriteBatch`.
/// Mirrors `clmain_write_t` in `Quake/cl_main_glue.c`.
///
/// `kind`: 0 = `MSG_WriteByte (sb, i)`, 1 = `MSG_WriteString (sb, p)`.
/// `cl_main.c` emits no other write width.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClMainWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub p: *const c_void,
}

extern "C" {
    /* Quake/cl_main_glue.c data -- cl_main.c:30-72, minus cl/cls. */

    pub static mut cl_name: cvar_t;
    pub static mut cl_topcolor: cvar_t;
    pub static mut cl_bottomcolor: cvar_t;
    pub static mut cl_shownet: cvar_t;
    pub static mut cl_nolerp: cvar_t;
    pub static mut cfg_unbindall: cvar_t;
    pub static mut lookstrafe: cvar_t;
    pub static mut sensitivity: cvar_t;
    pub static mut m_pitch: cvar_t;
    pub static mut m_yaw: cvar_t;
    pub static mut m_forward: cvar_t;
    pub static mut m_side: cvar_t;
    pub static mut cl_startdemos: cvar_t;

    /// `cl_main.c:53`. Header-less external: `menu.c` declares it locally.
    pub static mut cl_confirmquit: cvar_t;

    pub static mut cl_dlights: [dlight_t; 64];

    pub static mut cl_numvisedicts: c_int;
    pub static mut cl_numvisedicts_alpha_overwater: c_int;
    pub static mut cl_numvisedicts_alpha_underwater: c_int;
    pub static mut cl_maxvisedicts: c_int;
    pub static mut cl_visedicts: *mut *mut c_void;
    pub static mut cl_visedicts_alpha: *mut *mut c_void;

    /// `cl_main.c:72`. Header-less external: `view.c` declares it locally.
    pub static mut needs_relink: crate::qboolean;

    /* Header-less engine objects cl_main.c:69-71 declared at file scope. */

    /// `host.c:67`.
    pub static mut host_netinterval: c_float;
    /// `gl_rmisc.c`.
    pub static mut r_lerpmodels: cvar_t;
    /// `gl_rmisc.c`.
    pub static mut r_lerpturn: cvar_t;
    /// `render.h:201` / `glquake.h:551` -- the view forward vector.
    pub static mut vpn: [c_float; 3];

    /* Quake/cl_main_glue.c guards. Each returns a Host_Guard status. */

    /// Replays `count` buffered writes into `sb` inside one `Host_Guard`.
    /// Every `MSG_Write*` reaches `SZ_GetSpace` (`net_msg.c:481`), which
    /// `Host_Error`s on overflow, so no Rust frame may sit under one.
    /// `sb` is explicit because `SV_UpdateInfo` (`cl_main.c:1271`) targets a
    /// `client_t`'s message buffer, not `cls.message`.
    pub fn ClMain_Glue_WriteBatch(
        sb: *mut c_void,
        ops: *const ClMainWriteOp,
        count: c_int,
    ) -> c_int;

    /// `cl_main.c:80-90` -- `PScript_DelinkTrailstate (&ts)`.
    pub fn ClMain_Glue_DelinkTrailstate(ts: *mut c_void) -> c_int;

    /// `cl_main.c:99` -- `PR_ClearProgs (&cl.qcvm)`.
    pub fn ClMain_Glue_PRClearProgs(qcvm: *mut c_void) -> c_int;

    /// `cl_main.c:104`, `:109`, `:719` -- `R_FreeEntityBLAS (ent)`.
    pub fn ClMain_Glue_FreeEntityBLAS(ent: *mut c_void) -> c_int;

    /// `cl_main.c:133` -- `Host_ClearMemory ()`.
    pub fn ClMain_Glue_HostClearMemory() -> c_int;

    /// `cl_main.c:158` -- `PScript_Shutdown ()`.
    pub fn ClMain_Glue_PScriptShutdown() -> c_int;

    /// `cl_main.c:169` -- `Key_EndChat ()`.
    pub fn ClMain_Glue_KeyEndChat() -> c_int;

    /// `cl_main.c:172-174` -- `S_StopAllSounds (true, false)`, `BGM_Stop ()`
    /// and `CDAudio_Stop ()`, in source order. Grouped because the three are
    /// unconditional and adjacent and nothing between them is observable.
    pub fn ClMain_Glue_StopAudio() -> c_int;

    /// `cl_main.c:189` -- `NET_SendUnreliableMessage (cls.netcon,
    /// &cls.message)`. `*out` is cleared before the guarded call.
    pub fn ClMain_Glue_NetSendUnreliable(
        sock: *mut qsocket_s,
        sb: *mut sizebuf_t,
        out: *mut c_int,
    ) -> c_int;

    /// `cl_main.c:191` -- `NET_Close (cls.netcon)`.
    pub fn ClMain_Glue_NetClose(sock: *mut qsocket_s) -> c_int;

    /// `cl_main.c:196`, `:211` -- `Host_ShutdownServer (false)`.
    pub fn ClMain_Glue_HostShutdownServer(crash: c_int) -> c_int;

    /// `cl_main.c:203` -- `SCR_CenterPrintClear ()`.
    pub fn ClMain_Glue_CenterPrintClear() -> c_int;

    /// `cl_main.c:229` -- `NET_Connect (host)`. `*out` is cleared first.
    pub fn ClMain_Glue_NetConnect(host: *const c_char, out: *mut *mut qsocket_s) -> c_int;

    /// `cl_main.c:278` -- `Info_Enumerate (cls.userinfo,
    /// CL_SendInitialUserinfo, NULL)`. The callback is `cl_main_glue.c`'s own
    /// re-raising `CL_SendInitialUserinfo`, so a jump out of it lands in this
    /// guard one frame up.
    pub fn ClMain_Glue_InfoEnumerate(info: *const c_char) -> c_int;

    /// `cl_main.c:290` -- `SCR_EndLoadingPlaque ()`.
    pub fn ClMain_Glue_EndLoadingPlaque() -> c_int;

    /// `cl_main.c:319` -- `SCR_BeginLoadingPlaque ()`.
    pub fn ClMain_Glue_BeginLoadingPlaque() -> c_int;

    /// `cl_main.c:322` -- `Cbuf_InsertText (str)`.
    pub fn ClMain_Glue_CbufInsertText(text: *const c_char) -> c_int;

    /// `cl_main.c:680` -- `SCR_UpdateZoom ()`.
    pub fn ClMain_Glue_UpdateZoom() -> c_int;

    /// `cl_main.c:720` -- `InvalidateTraceLineCache ()`.
    pub fn ClMain_Glue_InvalidateTraceLineCache() -> c_int;

    /// `cl_main.c:757` -- `R_EntityParticles (ent)`.
    pub fn ClMain_Glue_EntityParticles(ent: *mut c_void) -> c_int;

    /// `cl_main.c:640` -- `R_RocketTrail (ent->trailorg, ent->origin, type)`.
    pub fn ClMain_Glue_RocketTrail(
        start: *const c_float,
        end: *const c_float,
        type_: c_int,
    ) -> c_int;

    /// `cl_main.c:925` -- `R_AllocateEntityBLAS (ent)`.
    pub fn ClMain_Glue_AllocateEntityBLAS(ent: *mut c_void) -> c_int;

    /// `cl_main.c:931` -- `R_UpdateEntityDlights ()`.
    pub fn ClMain_Glue_UpdateEntityDlights() -> c_int;

    /// `cl_main.c:844`, `:850` -- `PScript_ParticleTrail (start, end, type,
    /// timeinterval, dlkey, axis, tsk)`. `axis` is nine contiguous floats.
    pub fn ClMain_Glue_ParticleTrail(
        start: *const c_float,
        end: *const c_float,
        type_: c_int,
        timeinterval: c_float,
        dlkey: c_int,
        axis: *const c_float,
        tsk: *mut *mut c_void,
    ) -> c_int;

    /// `cl_main.c:857`, `:862`, ... -- `PScript_EntParticleTrail (oldorg, ent,
    /// name)`. `*out` is cleared before the guarded call.
    pub fn ClMain_Glue_EntParticleTrail(
        oldorg: *const c_float,
        ent: *mut c_void,
        name: *const c_char,
        out: *mut c_int,
    ) -> c_int;

    /// `cl_main.c:901`, `:913` -- `PScript_RunParticleEffectState (org, dir,
    /// count, typenum, tsk)`.
    pub fn ClMain_Glue_RunParticleEffectState(
        org: *const c_float,
        dir: *const c_float,
        count: c_float,
        typenum: c_int,
        tsk: *mut *mut c_void,
    ) -> c_int;

    /// `cl_main.c:948` -- `PScript_FindParticleType (name)`.
    pub fn ClMain_Glue_FindParticleType(name: *const c_char, out: *mut c_int) -> c_int;

    /// `cl_main.c:989` -- `CL_ParseServerMessage ()`.
    pub fn ClMain_Glue_ParseServerMessage() -> c_int;

    /// `cl_main.c:997` -- `CL_UpdateTEnts ()`.
    pub fn ClMain_Glue_UpdateTEnts() -> c_int;

    /// `cl_main.c:1054` -- `IN_Move (&cl.pendingcmd)`.
    pub fn ClMain_Glue_InMove(cmd: *mut c_void) -> c_int;

    /// `cl_main.c:1101` -- `NET_CanSendMessage (cls.netcon)`.
    pub fn ClMain_Glue_NetCanSendMessage(sock: *mut qsocket_s, out: *mut c_int) -> c_int;

    /// `cl_main.c:1107` -- `NET_SendMessage (cls.netcon, &cls.message)`.
    pub fn ClMain_Glue_NetSendMessage(
        sock: *mut qsocket_s,
        sb: *mut sizebuf_t,
        out: *mut c_int,
    ) -> c_int;

    /// `cl_main.c:1130` -- `TraceLine (r_refdef.vieworg, v, w)`.
    pub fn ClMain_Glue_TraceLine(
        start: *const c_float,
        end: *const c_float,
        impact: *mut c_float,
    ) -> c_int;

    /// `cl_main.c:1212` -- `R_TranslateNewPlayerSkin (sb - cl.scores)`.
    pub fn ClMain_Glue_TranslateNewPlayerSkin(slot: c_int) -> c_int;

    /// `cl_main.c:1268` -- `PR_SetEngineString (client->name)`.
    pub fn ClMain_Glue_PRSetEngineString(s: *const c_char, out: *mut c_int) -> c_int;

    /// `cl_main.c:1286` -- `Cvar_Set (var->name, value)`. Guarded because a
    /// cvar callback is arbitrary engine code.
    pub fn ClMain_Glue_CvarSet(name: *const c_char, value: *const c_char) -> c_int;

    /// `cl_main.c:1357-1358` -- `Cvar_SetValue (name, v)`. Same reason.
    pub fn ClMain_Glue_CvarSetValue(name: *const c_char, value: c_float) -> c_int;

    /// `cl_main.c:1376` etc -- `Cvar_RegisterVariable (var)`. Under
    /// `-Duse_rust_cvar` the plain name is itself a `Host_Reraise` wrapper,
    /// so it is guarded here for the same reason `chase_glue.c` guards it.
    pub fn ClMain_Glue_RegisterVariable(var: *mut cvar_t) -> c_int;

    /* Quake/cl_main_glue.c plain shims. None of these can raise. */

    /// `cl_main.c:1414-1416` -- `cmd->completion = CL_Viewpos_Completion_f`.
    /// `cmd_function_s` is opaque to bindgen (`generated.rs:658-661`), so the
    /// field write has to happen in C.
    pub fn ClMain_Glue_SetViewposCompletion(cmd: *mut c_void);

    /// `cl_main.c:1173` -- `SDL_SetClipboardText (buf)`.
    pub fn ClMain_Glue_SetClipboardText(text: *const c_char);

    /// `cl_main.c:1272-1277` -- `Cvar_FindVar (keyname)` plus the
    /// `CVAR_SERVERINFO` test, so the port never has to know the layout of a
    /// `cvar_t` it does not own. Returns 1 and writes `var->name` when the
    /// name resolved to a `CVAR_SERVERINFO` cvar, 0 otherwise.
    pub fn ClMain_Glue_FindServerinfoCvar(
        keyname: *const c_char,
        out_name: *mut *const c_char,
    ) -> c_int;

    /* Non-raising engine C entry points. */

    /// `console.h:65`.
    pub fn Con_AddToTabList(name: *const c_char, partial: *const c_char, type_: *const c_char);

    /// `common.h:200`.
    pub fn Info_GetKey(
        info: *const c_char,
        key: *const c_char,
        out: *mut c_char,
        outsize: usize,
    ) -> *const c_char;

    /// `common.h` -- rotating static formatter.
    pub fn va(format: *const c_char, ...) -> *mut c_char;

    /// `q_stdinc.h`.
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;

    /* libc */
    pub fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
    pub fn strncpy(dst: *mut c_char, src: *const c_char, n: usize) -> *mut c_char;
    pub fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    pub fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong;
}

/// `Quake/glquake.h:613` -- re-exported so `quake-capi`'s `cl_main` reaches
/// the same declaration `cl_parse` uses instead of redeclaring it.
pub use crate::cl_parse::{dev_peakstats, dev_stats};

/// `cl_main.c:58` -- `lightstyle_t cl_lightstyle[MAX_LIGHTSTYLES]`, owned by
/// `cl_main_glue.c` and already declared by [`crate::cl_parse`].
pub use crate::cl_parse::cl_lightstyle;

/// Kept so the module's `lightstyle_t`/`devstats_t` imports are used even when
/// only the re-exports above are consumed.
const _: () = {
    assert!(core::mem::size_of::<lightstyle_t>() > 0);
    assert!(core::mem::size_of::<devstats_t>() == 28);
};
