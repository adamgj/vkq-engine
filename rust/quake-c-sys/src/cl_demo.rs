//! `Quake/cl_demo_glue.c` declarations (Rust migration Phase 7 M7, T7.4).
//!
//! ADR-011: engine C symbols are declared only in this crate. `cl_demo.c`
//! defined no C-visible object -- its two file-scope objects (`name` at
//! `cl_demo.c:37` and `weirdaltbufferthatprobablyisntneeded` at `:584`) had
//! internal linkage, so they became Rust statics -- and all eight of its
//! non-static functions are declared in `client.h:404-412`. The glue file
//! therefore contributes only the thirteen `Host_Guard` trampolines below
//! (ADR-009).
//!
//! `cl`/`cls` are mirror-typed, so they are declared in
//! `quake-capi/src/cl_demo.rs`, which can name `quake_types`; this crate has
//! no `[dependencies]`.
//!
//! `Host_Reraise` is deliberately absent: only `cl_demo_glue.c` calls it
//! (ADR-009 rule 3). A `ClDemo_Glue_*` returning non-zero is propagated
//! upward as a status and re-issued from that pure-C frame.

use crate::{cmd_source_t, qboolean, qsocket_s, FILE};
use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

/// One buffered `MSG_Write*` op replayed by `ClDemo_Glue_WriteBatch`.
/// Mirrors `cldemo_write_t` in `Quake/cl_demo_glue.c`.
///
/// `kind`: 0 byte, 1 short, 2 long, 3 float, 4 string, 5 coord.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClDemoWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub f: c_float,
    pub u: c_uint,
    pub p: *const c_void,
}

extern "C" {
    /* Quake/cl_demo_glue.c guards -- each returns a Host_Guard status. */

    /// Replays `count` buffered writes into `net_message` inside one
    /// `Host_Guard`. Every `MSG_Write*` reaches `SZ_GetSpace`
    /// (`net_msg.c:481`), which `Host_Error`s on overflow.
    pub fn ClDemo_Glue_WriteBatch(ops: *const ClDemoWriteOp, count: c_int) -> c_int;

    /// `cl_demo.c:391`, `:400` -- `MSG_WriteStaticOrBaseLine (&net_message,
    /// idx, state, ...)`. Separate from the batch because it takes five
    /// operands; callers flush first so the byte order is preserved.
    pub fn ClDemo_Glue_WriteStaticOrBaseLine(
        idx: c_int,
        state: *mut c_void,
        pext2: c_uint,
        protocol: c_uint,
        protocolflags: c_uint,
    ) -> c_int;

    /// `cl_demo.c:302` -- `NET_GetMessage (cls.netcon)`. `net_main.c:29-34`
    /// documents the dispatch funnels as sitting above `Host_Error`-capable
    /// driver code. `*out` is cleared before the guarded call.
    pub fn ClDemo_Glue_NetGetMessage(sock: *mut qsocket_s, out: *mut c_int) -> c_int;

    /// `cl_demo.c:681` -- `Cmd_ExecuteString (va ("map %s", ...), src_command)`.
    pub fn ClDemo_Glue_CmdExecuteString(text: *const c_char, src: c_int) -> c_int;

    /// `cl_demo.c:767` -- `CL_Disconnect ()`, which reaches
    /// `Host_ShutdownServer` and the `ClientDisconnect` QC call.
    pub fn ClDemo_Glue_Disconnect() -> c_int;

    /// `cl_demo.c:260-267` -- `V_ResetBlend`, `Fog_NewMap`, `Sky_NewMap`,
    /// `R_ClearParticles`, `PScript_ClearParticles (false)` and
    /// `SCR_CenterPrintClear`, in source order. Grouped into one trampoline
    /// because the calls are unconditional, adjacent, and nothing between
    /// them is observable from Rust.
    pub fn ClDemo_Glue_SeekEffects() -> c_int;

    /// `cl_demo.c:271` -- `BGM_Stop ()`.
    pub fn ClDemo_Glue_BgmStop() -> c_int;

    /// `cl_demo.c:278` -- `S_StopAllSounds (true, true)`.
    pub fn ClDemo_Glue_StopAllSounds() -> c_int;

    /// `cl_demo.c:350` -- `DemoList_Rebuild ()`.
    pub fn ClDemo_Glue_DemoListRebuild() -> c_int;

    /// `cl_demo.c:426` -- `S_LoadSound (ss->sfx)`. `*out` is cleared before
    /// the guarded call.
    pub fn ClDemo_Glue_LoadSound(sfx: *mut c_void, out: *mut *mut c_void) -> c_int;

    /// `cl_demo.c:515` -- `Fog_GetFogCommand (false)`.
    pub fn ClDemo_Glue_FogGetFogCommand(out: *mut *const c_char) -> c_int;

    /// `cl_demo.c:522` -- `Sky_GetSkyCommand (false)`.
    pub fn ClDemo_Glue_SkyGetSkyCommand(out: *mut *const c_char) -> c_int;

    /// `cl_demo.c:775` -- `COM_FOpenFile (name, &cls.demofile, NULL)`.
    pub fn ClDemo_Glue_ComFOpenFile(name: *const c_char, file: *mut *mut FILE) -> c_int;

    /* Engine C symbols cl_demo.c calls directly. None of these can
    Host_Error / Host_EndGame. */

    /// `gl_screen.c:154`. Header-less: `cl_demo.c:213` declares
    /// `extern float scr_clock_off;` locally at the point of use, so the
    /// object stays C-owned and is reached from Rust the same way.
    pub static mut scr_clock_off: c_float;

    /// `harness.h:57` -- flushes hashes and `exit(0)`s; it never returns to a
    /// Rust frame by longjmp.
    pub fn Harness_DemoEnded();

    /// `console.h:47`.
    pub fn Con_LinkPrintf(addr: *const c_char, fmt: *const c_char, ...);

    /// `common.h` -- rotating static formatter.
    pub fn va(format: *const c_char, ...) -> *mut c_char;

    /// `q_stdinc.h`.
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;

    /// `host.c` -- `double realtime`.
    pub static mut realtime: f64;

    /// `keys.h` -- `keydest_t key_dest;`. `key_game == 0`
    /// (`keys.h:136-142`).
    pub static mut key_dest: c_int;

    /// `cmd.h` -- owned by `cvar_cmd_glue.c` since M2.
    pub static mut cmd_source: cmd_source_t;

    /* libc */
    pub fn fflush(stream: *mut FILE) -> c_int;
    pub fn fscanf(stream: *mut FILE, format: *const c_char, ...) -> c_int;
    pub fn sscanf(s: *const c_char, format: *const c_char, ...) -> c_int;
    pub fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
    pub fn atoi(s: *const c_char) -> c_int;
}

/// `client.h:412` -- `void CL_Resume_Record (qboolean recordsignons)`.
/// `q_types.h:122` makes `qboolean` C11 `bool` (one byte), which
/// `generated.rs:38` mirrors as Rust `bool`; this alias exists only so the
/// port's signature reads like the C.
pub type ClDemoQBoolean = qboolean;
