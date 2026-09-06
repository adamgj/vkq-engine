//! `Quake/host_glue.c` declarations (Rust migration Phase 7 M8, T8.2).
//!
//! ADR-011: engine C symbols are declared only in this crate. `host.c` defined
//! forty-two C-visible data objects; all of them keep C storage in
//! `Quake/host_glue.c` -- `host_abortserver` and `screen_error` are the ADR-009
//! longjmp targets, `host_client` and the `cvar_t`s are addressed from fourteen
//! other translation units, and `dev_stats`/`dev_peakstats`/`dev_overflows` are
//! shared with `cl_parse.c` and `sv_send.c` -- so no ADR-007 row opens or closes
//! at T8.2.
//!
//! Only the twenty-six objects that had no Rust declaration before T8.2 are
//! declared here; the rest already live in `sv_main`, `sv_phys`, `cl_main`,
//! `cl_demo`, `cl_parse`, `progs_builtins_sv` or `generated`.
//!
//! `pr_engine` (`host.c:98`) lost its `static` in the glue: `Host_InitLocal`,
//! its only address-taker, moved to Rust (the T7.4 linkage rule).
//!
//! Eight of the objects `host.c` reads or writes have external linkage but no
//! header declaration -- `screen_error`, `host_netinterval` (`host.c:63-67`),
//! `sv_speeds`, `host_maxfps`, `host_timescale`, `pausable`, `autoload`,
//! `autofastload` -- plus the seven `sv_speeds_*` counters `Host_ServerFrame`
//! declared at block scope (`host.c:900-901`). The port reaches them the same
//! way, and their storage stays C-side.
//!
//! `Host_Reraise` is deliberately absent: only `host_glue.c` calls it (ADR-009
//! rule 3). Each `Host_Glue_*` below returns a `Host_Guard` status (0 =
//! returned normally, 1 = `Host_Error`/`Host_EndGame`, 2 = `screen_error`)
//! which the Rust core propagates upward untouched.

use crate::cl_parse::devstats_t;
use crate::{cvar_t, qboolean};
use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

/// `quakedef.h:225-234` -- `quakeparms_t`. Declared here rather than reached
/// through a glue accessor because `Host_Init` reads two of its fields
/// directly (`host.c:1293-1294`); `quakever.h` and the rest of `quakedef.h`
/// are not bindgen-clean, so the mirror is hand-written (ADR-011).
#[repr(C)]
pub struct quakeparms_t {
    pub basedir: *const c_char,
    pub userdir: *const c_char,
    pub argc: c_int,
    pub argv: *mut *mut c_char,
    pub errstate: c_int,
}

/// One buffered `MSG_Write*` op replayed by `Host_Glue_WriteBatch`.
/// Mirrors `host_write_t` in `Quake/host_glue.c`.
///
/// `kind`: 0 = `MSG_WriteByte (sb, i)`, 1 = `MSG_WriteShort (sb, i)`,
/// 2 = `MSG_WriteString (sb, p)`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub p: *const c_void,
}

/// `Host_Guard` returned normally.
pub const HOST_GUARD_OK: c_int = 0;

extern "C" {
    /* Quake/host_glue.c data -- host.c:48-108, minus the sixteen objects
     * already declared elsewhere in this crate. */

    pub static mut host_parms: *mut quakeparms_t;
    pub static mut host_rawframetime: f64;
    pub static mut oldrealtime: f64;
    pub static mut minimum_memory: c_int;
    pub static mut host_colormap: *mut u8;

    pub static mut host_framerate: cvar_t;
    pub static mut host_speeds: cvar_t;
    pub static mut host_maxfps: cvar_t;
    pub static mut host_phys_max_ticrate: cvar_t;
    pub static mut host_timescale: cvar_t;
    pub static mut cl_nocsqc: cvar_t;
    pub static mut sys_ticrate: cvar_t;
    pub static mut serverprofile: cvar_t;
    pub static mut fraglimit: cvar_t;
    pub static mut timelimit: cvar_t;
    pub static mut samelevel: cvar_t;
    pub static mut noexit: cvar_t;
    pub static mut pausable: cvar_t;
    pub static mut autoload: cvar_t;
    pub static mut autofastload: cvar_t;
    pub static mut pr_engine: cvar_t;
    pub static mut temp1: cvar_t;
    pub static mut devstats: cvar_t;
    pub static mut campaign: cvar_t;
    pub static mut horde: cvar_t;
    pub static mut sv_cheats: cvar_t;

    /* host.c:107-108 -- shared with cl_parse.c and sv_send.c. */
    pub static mut dev_peakstats: devstats_t;

    /* host.c:900-901 -- sv_phys.c counters declared at block scope, with
     * external linkage and no header. */
    pub static mut sv_speeds_think_ms: f64;
    pub static mut sv_speeds_pusher_ms: f64;
    pub static mut sv_speeds_build_ms: f64;
    pub static mut sv_speeds_thinks: c_int;
    pub static mut sv_speeds_pushers: c_int;
    pub static mut sv_speeds_pushables: c_int;
    pub static mut sv_speeds_grid_entries: c_int;

    /* Preprocessor-only values the glue owns on the port's behalf. */
    pub static host_glue_version: f64;
    pub static host_glue_quakespasm_ver: *const c_char;
    pub static host_glue_engine_ver: *const c_char;
    pub static host_glue_build_time: *const c_char;
    pub static host_glue_build_date: *const c_char;
    pub static host_glue_build_suffix: *const c_char;

    /* host.c:1277-1284 -- Tests_Init's registration table. */
    pub static host_glue_test_names: [*const c_char; 3];
    pub static host_glue_test_funcs: [Option<unsafe extern "C" fn()>; 3];
    pub static host_glue_num_tests: c_int;

    /* Engine globals host.c reads that had no Rust declaration. */
    pub static mut no_rendering: qboolean;
    pub static mut scr_disabled_for_loading: qboolean;
    pub static mut con_initialized: qboolean;
    pub static mut pr_csqcnumbuiltins: c_int;
    pub static mut r_origin: [c_float; 3];
    pub static mut vright: [c_float; 3];
    pub static mut vup: [c_float; 3];

    /// `Quake/pr_cmds.c` -- `builtin_t pr_csqcbuiltins[]`, passed straight
    /// back to `PR_LoadProgs` and never indexed from Rust.
    pub static pr_csqcbuiltins: [u8; 0];
}

extern "C" {
    /* Guarded seams (ADR-009 rule 3). Each returns a Host_Guard status. */

    pub fn Host_Glue_Mod_ClearAll() -> c_int;
    pub fn Host_Glue_Sky_ClearAll() -> c_int;
    pub fn Host_Glue_S_ClearAll() -> c_int;
    pub fn Host_Glue_SV_ClearWorld() -> c_int;
    pub fn Host_Glue_SV_ClearDatagram() -> c_int;
    pub fn Host_Glue_SV_CheckForNewClients() -> c_int;
    pub fn Host_Glue_SV_RunClients() -> c_int;
    pub fn Host_Glue_SV_Physics() -> c_int;
    pub fn Host_Glue_SV_SendClientMessages() -> c_int;
    pub fn Host_Glue_Key_UpdateForDest() -> c_int;
    pub fn Host_Glue_IN_UpdateInputMode() -> c_int;
    pub fn Host_Glue_IN_Commands() -> c_int;
    pub fn Host_Glue_Con_UpdateMouseState() -> c_int;
    pub fn Host_Glue_Cbuf_Execute() -> c_int;
    pub fn Host_Glue_NET_Poll() -> c_int;
    pub fn Host_Glue_CL_AccumulateCmd() -> c_int;
    pub fn Host_Glue_M_UpdateMouse() -> c_int;
    pub fn Host_Glue_CL_SendCmd() -> c_int;
    pub fn Host_Glue_CL_ReadFromServer() -> c_int;
    pub fn Host_Glue_CL_RunParticles() -> c_int;
    pub fn Host_Glue_CL_DecayLights() -> c_int;
    pub fn Host_Glue_BGM_Update() -> c_int;
    pub fn Host_Glue_CDAudio_Update() -> c_int;
    pub fn Host_Glue_Harness_Frame() -> c_int;
    pub fn Host_Glue_CL_Disconnect() -> c_int;
    pub fn Host_Glue_CL_FreeState() -> c_int;
    pub fn Host_Glue_Host_InitCommands() -> c_int;
    pub fn Host_Glue_Mem_Init() -> c_int;
    pub fn Host_Glue_Tasks_Init() -> c_int;
    pub fn Host_Glue_Cbuf_Init() -> c_int;
    pub fn Host_Glue_Cmd_Init() -> c_int;
    pub fn Host_Glue_Cvar_Init() -> c_int;
    pub fn Host_Glue_COM_Init() -> c_int;
    pub fn Host_Glue_COM_InitFilesystem() -> c_int;
    pub fn Host_Glue_W_LoadWadFile() -> c_int;
    pub fn Host_Glue_Key_Init() -> c_int;
    pub fn Host_Glue_Con_Init() -> c_int;
    pub fn Host_Glue_PR_Init() -> c_int;
    pub fn Host_Glue_Mod_Init() -> c_int;
    pub fn Host_Glue_NET_Init() -> c_int;
    pub fn Host_Glue_SV_Init() -> c_int;
    pub fn Host_Glue_V_Init() -> c_int;
    pub fn Host_Glue_Chase_Init() -> c_int;
    pub fn Host_Glue_M_Init() -> c_int;
    pub fn Host_Glue_ExtraMaps_Init() -> c_int;
    pub fn Host_Glue_M_CheckMods() -> c_int;
    pub fn Host_Glue_Modlist_Init() -> c_int;
    pub fn Host_Glue_DemoList_Init() -> c_int;
    pub fn Host_Glue_SaveList_Init() -> c_int;
    pub fn Host_Glue_VID_Init() -> c_int;
    pub fn Host_Glue_IN_Init() -> c_int;
    pub fn Host_Glue_TexMgr_Init() -> c_int;
    pub fn Host_Glue_Draw_Init() -> c_int;
    pub fn Host_Glue_SCR_Init() -> c_int;
    pub fn Host_Glue_R_Init() -> c_int;
    pub fn Host_Glue_S_Init() -> c_int;
    pub fn Host_Glue_CDAudio_Init() -> c_int;
    pub fn Host_Glue_BGM_Init() -> c_int;
    pub fn Host_Glue_Sbar_Init() -> c_int;
    pub fn Host_Glue_R_InitParticles() -> c_int;
    pub fn Host_Glue_LOC_Init() -> c_int;
    pub fn Host_Glue_Harness_Init() -> c_int;
    pub fn Host_Glue_COM_WriteSelectedBaseDir() -> c_int;
    pub fn Host_Glue_CL_Init() -> c_int;
    pub fn Host_Glue_PScript_InitParticles() -> c_int;
    pub fn Host_Glue_PR_TraceInit() -> c_int;
    pub fn Host_Glue_PR_TraceShutdown() -> c_int;
    pub fn Host_Glue_Harness_Shutdown() -> c_int;
    pub fn Host_Glue_NET_Shutdown() -> c_int;
    pub fn Host_Glue_History_Shutdown() -> c_int;
    pub fn Host_Glue_ExtraMaps_ShutDown() -> c_int;
    pub fn Host_Glue_BGM_Shutdown() -> c_int;
    pub fn Host_Glue_CDAudio_Shutdown() -> c_int;
    pub fn Host_Glue_S_Shutdown() -> c_int;
    pub fn Host_Glue_IN_Shutdown() -> c_int;
    pub fn Host_Glue_VID_Shutdown() -> c_int;
    pub fn Host_Glue_Steam_Shutdown() -> c_int;
    pub fn Host_Glue_LOG_Close() -> c_int;
    pub fn Host_Glue_LOC_Shutdown() -> c_int;

    /* Pointer- and int-operand seams. */
    pub fn Host_Glue_LOG_Init(parms: *mut c_void) -> c_int;
    pub fn Host_Glue_PR_ClearProgs(vm: *mut c_void) -> c_int;
    pub fn Host_Glue_Key_WriteBindings(f: *mut c_void) -> c_int;
    pub fn Host_Glue_Cvar_WriteVariables(f: *mut c_void) -> c_int;
    pub fn Host_Glue_SVFTE_DestroyFrames(client: *mut c_void) -> c_int;
    pub fn Host_Glue_NET_Close(sock: *mut c_void) -> c_int;
    pub fn Host_Glue_SCR_UpdateScreen(clear: c_int) -> c_int;
    pub fn Host_Glue_PR_ExecuteProgram(func: c_int) -> c_int;

    /* Value-returning seams. The result is written through an out-parameter so
     * the int return stays the Host_Guard status. */
    pub fn Host_Glue_CvarSetQuick(var: *mut c_void, value: *const c_char) -> c_int;
    pub fn Host_Glue_CbufAddText(text: *const c_char) -> c_int;
    pub fn Host_Glue_CbufInsertText(text: *const c_char) -> c_int;
    pub fn Host_Glue_PRLoadProgs(
        filename: *const c_char,
        needcrc: c_uint,
        builtins: *const c_void,
        numbuiltins: usize,
        out: *mut c_int,
    ) -> c_int;
    pub fn Host_Glue_PRSetEngineString(s: *const c_char, out: *mut c_int) -> c_int;
    pub fn Host_Glue_EdictNum(n: c_int, out: *mut *mut c_void) -> c_int;
    pub fn Host_Glue_EdictToProg(e: *mut c_void, out: *mut c_int) -> c_int;
    pub fn Host_Glue_NetCanSendMessage(sock: *mut c_void, out: *mut c_int) -> c_int;
    pub fn Host_Glue_NetSendMessage(sock: *mut c_void, data: *mut c_void, out: *mut c_int)
        -> c_int;
    pub fn Host_Glue_NetGetMessage(sock: *mut c_void, out: *mut c_int) -> c_int;
    pub fn Host_Glue_ComLoadFile(path: *const c_char, out: *mut *mut c_void) -> c_int;
    pub fn Host_Glue_SUpdate(
        origin: *const c_float,
        forward: *const c_float,
        right: *const c_float,
        up: *const c_float,
    ) -> c_int;

    /// Replays `count` buffered writes against `sb`. host.c writes into four
    /// different sizebufs, so the target is explicit.
    pub fn Host_Glue_WriteBatch(sb: *mut c_void, ops: *const HostWriteOp, count: c_int) -> c_int;

    /// `host.c:702-710` -- the stack-local `sizebuf_t`/`byte[4]` disconnect
    /// broadcast, kept whole in C because the buffer must not outlive the call.
    pub fn Host_Glue_BroadcastDisconnect(out_count: *mut c_int) -> c_int;

    /// `host.c:1085-1094` -- the `_Host_Frame` setjmp shell. Swallows a caught
    /// raise exactly as the C build's early `return` did, so it has no status.
    pub fn Host_Glue_FrameInner(time: f64);

    /* Plain C callees host.c reaches that had no Rust declaration. All are
     * raise-free, so they are called straight through (ADR-009 rule 4). */
    pub fn Cbuf_Waited();
    pub fn Sys_ConsoleInput() -> *const c_char;
    pub fn Sys_SendKeyEvents();
    pub fn SDL_Delay(ms: u32);
    pub fn Steam_SetStatus_Menu();
    pub fn Steam_SetStatus_SinglePlayer(map: *const c_char);
    pub fn Steam_SetStatus_Multiplayer(players: c_int, maxplayers: c_int, map: *const c_char);

    /// `Quake/mathlib.c:27` -- the shared zero vector `_Host_Frame` passes to
    /// `S_Update` four times when rendering is disabled (`host.c:1216`).
    pub static vec3_origin: [c_float; 3];

    /* The two `host.c` entry points that are also registered as C callbacks.
     * `Cmd_AddCommand`/`Cvar_SetCallback` must receive the glue wrapper, not
     * the Rust core, so that a raise inside them unwinds through C. */
    pub fn Host_Version_f();
    pub fn Host_Callback_Notify(var: *mut cvar_t);

    /* libc, as `host.c` used it. */
    pub fn atoi(s: *const c_char) -> c_int;
    pub fn strtoul(s: *const c_char, end: *mut *mut c_char, base: c_int) -> core::ffi::c_ulong;
    pub fn printf(fmt: *const c_char, ...) -> c_int;
}

// Phase 8 M2: the task-system queries live in `crate::tasks` (ADR-016).
pub use crate::tasks::Tasks_IsWorker;
