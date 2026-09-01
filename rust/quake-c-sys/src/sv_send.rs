//! `Quake/sv_send_glue.c` declarations (Rust migration Phase 7 M6, T6.3).
//!
//! ADR-011: engine C symbols are declared only in this crate. Unlike its
//! siblings this glue owns *no* C-visible storage: `Quake/sv_send.c` defines
//! no objects at all -- its only file-scope declarations are the two
//! `extern cvar_t`s at `sv_send.c:27-28` (`sv_netsort`,
//! `sv_smoothplatformlerps`), both of which are defined by
//! `Quake/sv_main_glue.c:69-70` (T6.5). Everything below is either an
//! ADR-009 `Host_Guard` trampoline or a shim that keeps a renamed or
//! macro-heavy fragment on the C side.
//!
//! ## ADR-009 raise topology
//!
//! Guarded, each returning a `Host_Guard` status that must reach the caller
//! untouched: every `MSG_Write*` and `SZ_Write` run (batched, see
//! `SvSend_Glue_WriteBatch`), `PR_GetString`, `SV_SetIdealPitch`,
//! `SV_DropClient` and `Cmd_ExecuteString`.
//!
//! Not guarded, and why: `Sys_Error` (`sv_send.c:1096`) aborts rather than
//! longjmping; `Mem_Alloc`, `Mem_Realloc`, `Mem_Free`, `q_strdup`,
//! `Mod_LeafPVS`, `ED_FindFieldOffset`, `GetEdictFieldValue`, `SZ_Clear`,
//! `AngleVectors` and the `Con_*` printers have no `Host_Error` path; and
//! the `NET_Send*`, `NET_CanSendMessage` and `NET_SendToAll` funnels reach
//! only `Sys_Error` in the loopback and datagram drivers
//! (`net_loop.c:183`, `net_dgrm.c:344`, `net_dgrm.c:663`).
//!
//! ## Finding: `quake-c-sys` cannot depend on `quake-types`
//!
//! `rust/quake-c-sys/Cargo.toml` still has no `[dependencies]` section, so
//! this crate cannot name `quake_types::host::Server` (or `Client`,
//! `ClientStatic`, `EntityState`). The mirror-typed externs `sv`, `svs`,
//! `cls`, `host_client` and `nullentitystate` are therefore declared in
//! `quake-capi/src/sv_send.rs` instead. T6.4 (`sv_user.rs`) and T6.5
//! (`sv_main.rs`) recorded the identical finding; this is the third
//! occurrence, reported rather than worked around silently.
//!
//! Engine aggregates whose layout this file does not need are passed as
//! `c_void` pointers, matching the M4 (`sv_phys.rs`) convention.

use crate::qboolean;
use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

extern "C" {
    /* Engine C data sv_send.c reads. Not renamed by the ctest prelude, so
    both sides of the differential share this storage. */

    /// `Quake/gl_rmisc.c` -- `SV_UsePredThinkPos`'s block-scope
    /// `extern cvar_t r_lerpmove;` (`sv_send.c:36`).
    pub static mut r_lerpmove: crate::cvar_t;

    /* Non-raising engine C entry points. */

    /// `Quake/net.h` -- the outgoing reliable sequence number
    /// (`sv_send.c:538`, `sv_send.c:703`). Reads one socket field.
    pub fn NET_QSocketGetSequenceOut(sock: *mut c_void) -> c_int;
    /// `Quake/net.h` -- `sv_send.c:270`'s `"LOCAL"` test. Returns a pointer
    /// to a static buffer.
    pub fn NET_QSocketGetTrueAddressString(sock: *const c_void) -> *const c_char;
    /// `Quake/net.h` (`net_main.c:840`). Only `Sys_Error` lies beneath it.
    pub fn NET_SendUnreliableMessage(sock: *mut c_void, data: *mut c_void) -> c_int;
    /// `Quake/net.h` (`net_main.c:890`), used by `SV_SendReconnect`
    /// (`sv_send.c:2271`). Drives `NET_SendMessage` and `NET_GetMessage` in
    /// a loop; still no `Host_Error` beneath it.
    pub fn NET_SendToAll(data: *mut c_void, blocktime: f64) -> c_int;

    /// C runtime -- `SVFTE_WriteStats`'s old/new stat-string compare
    /// (`sv_send.c:589`).
    pub fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    /// C runtime -- the `"miss"`, `"rocket"` and `"gib"` model-name priority
    /// heuristics (`sv_send.c:1238-1247`).
    pub fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
    /// C runtime -- the prespawn precache budget checks (`sv_send.c:1953`).
    pub fn strlen(s: *const c_char) -> usize;

    /* Quake/sv_send_glue.c guards. Each returns a Host_Guard status. */

    /// Executes `ops[0 .. count]` against `sb` inside one `Host_Guard`
    /// frame. Every `MSG_Write*` and `SZ_Write` reaches `SZ_GetSpace`,
    /// which `Host_Error`s when the sizebuf disallows overflow
    /// (`net_msg.c:488`), so no such call may be made straight from Rust.
    ///
    /// Batching matters for correctness, not just for call overhead:
    /// `sv_send.c` reads `msg->cursize` between writes (rollback points,
    /// packet budgets), so the Rust `Writer` must flush before every such
    /// read. See `quake-capi/src/sv_send.rs`.
    pub fn SvSend_Glue_WriteBatch(
        sb: *mut c_void,
        ops: *const SvSendWriteOp,
        count: c_int,
    ) -> c_int;

    /// `PR_GetString` (`sv_send.c:66`, `:132`, `:890`, `:1215`, `:1551`,
    /// `:2244`) -- `Host_Error`s on a handle outside the string table.
    pub fn SvSend_Glue_GetString(handle: c_int, out: *mut *const c_char) -> c_int;

    /// `SV_SetIdealPitch ()` (`sv_send.c:1538`). Under `-Duse_rust_host`
    /// the plain name is itself T6.4's `Host_Reraise` wrapper, so a Rust
    /// frame must never call it directly.
    pub fn SvSend_Glue_SetIdealPitch() -> c_int;

    /// `SV_DropClient (crash)` (`sv_send.c:1873`, `:1932`, `:2157`).
    /// A confirmed transitive raise site through `host.c:590`'s
    /// `PR_ExecuteProgram (pr_global_struct->ClientDisconnect)`.
    pub fn SvSend_Glue_DropClient(crash: qboolean) -> c_int;

    /// `Cmd_ExecuteString ("reconnect\n", src_command)` (`sv_send.c:2274`).
    /// The command string and `src_command` stay in C.
    pub fn SvSend_Glue_ExecuteReconnect() -> c_int;

    /* Quake/sv_send_glue.c non-raising shims. */

    /// `SZ_Clear (sb)` (`sv_send.c:1759`, `:1806`, ...). Two field stores,
    /// so it cannot raise; routed through the glue only so that both sides
    /// of the ctest differential run the same `net_msg.c` body -- the
    /// oracle prelude renames `SZ_Clear` (`c_ref_prelude.h:91`).
    pub fn SvSend_Glue_SzClear(sb: *mut c_void);

    /// `AngleVectors (clent->v.v_angle, forward, right, up)`
    /// (`sv_send.c:1168`). Renamed by the prelude
    /// (`c_ref_prelude.h:225`), so it goes through the glue for the same
    /// reason as `SZ_Clear`.
    pub fn SvSend_Glue_AngleVectors(
        angles: *const c_float,
        forward: *mut c_float,
        right: *mut c_float,
        up: *mut c_float,
    );

    /// `standard_quake` (`sv_send.c:1667`). Renamed by the prelude
    /// (`c_ref_prelude.h:402`); an accessor keeps the two differential
    /// sides reading the same object.
    pub fn SvSend_Glue_StandardQuake() -> c_int;

    /// `SV_ModelIndex (name)` (`sv_send.c:67`, `:1551`, `:2242`). Cannot
    /// raise -- its only error path is `Sys_Error` -- but it is renamed by
    /// the prelude (`c_ref_prelude.h:1144`), so the shim keeps both
    /// differential sides on one implementation. In the real build it
    /// forwards to the plain name, which is Rust
    /// (`quake-capi/src/sv_main.rs`).
    pub fn SvSend_Glue_ModelIndex(name: *const c_char) -> c_int;

    /// `sv_player = ent;` (`sv_send.c:1763`). The prelude renames
    /// `sv_player` and T6.4's `sv_user_ref.c` defines the plain object, so
    /// the store must land on whichever side of that split the glue was
    /// compiled for.
    pub fn SvSend_Glue_SetPlayer(ent: *mut c_void);

    /// `sv_send.c:270`'s `#ifdef LERP_BANDAID` strip test:
    /// `cls.demorecording || strcmp (NET_QSocketGetTrueAddressString
    /// (host_client->netconnection), "LOCAL")`. Kept in C because it reads
    /// the client-side `cls` from inside the server writer and sits under a
    /// compile-time condition.
    pub fn SvSend_Glue_StripLerp() -> c_int;

    /// `Con_DWarning ("%i byte packet exceeds standard limit of 1024.\n",
    /// msg->cursize)` (`sv_send.c:800`). A shim so the format string is not
    /// duplicated in Rust.
    pub fn SvSend_Glue_WarnPacket(cursize: c_int);

    /// `Con_DWarning ("%i byte packet exceeds standard limit of 1024 (max =
    /// %d).\n", msg->cursize, msg->maxsize)` (`sv_send.c:1465`).
    pub fn SvSend_Glue_WarnPacketMax(cursize: c_int, maxsize: c_int);

    /// `Con_Printf ("Packet overflow!\n")` (`sv_send.c:1452`).
    pub fn SvSend_Glue_WarnOverflow();

    /// `Sys_Error ("SV_FatPVS: realloc() failed on %d bytes",
    /// fatpvs_capacity)` (`sv_send.c:1096`). Terminates rather than
    /// longjmping, so this is not a `Host_Guard` site (ADR-009); it stays
    /// in C only to keep the format string out of Rust.
    pub fn SvSend_Glue_FatPvsAllocFailed(capacity: c_int) -> !;

    /* Other non-raising engine C entry points, not renamed by the prelude. */

    /// `Quake/gl_model.h:741` -- `byte *Mod_LeafPVS (mleaf_t *leaf,
    /// qmodel_t *model)` (`sv_send.c:1058`). Pure lookup plus an optional
    /// decompress into a static buffer.
    pub fn Mod_LeafPVS(leaf: *mut c_void, model: *mut c_void) -> *mut u8;
}

/// One buffered write for `SvSend_Glue_WriteBatch`. Must stay
/// layout-identical to `svsend_write_t` in `Quake/sv_send_glue.c`.
///
/// `kind` selects the writer: 0 `MSG_WriteByte(i)`, 1 `MSG_WriteChar(i)`,
/// 2 `MSG_WriteShort(i)`, 3 `MSG_WriteLong(i)`, 4 `MSG_WriteFloat(f)`,
/// 5 `MSG_WriteString(p)`, 6 `MSG_WriteCoord(f, u)`, 7 `MSG_WriteAngle(f,
/// u)`, 8 `MSG_WriteAngle16(f, u)`, 9 `MSG_WriteEntity(i, u)`,
/// 10 `SZ_Write(p, i)`.
///
/// `u` carries the protocol flag word per op rather than being read from
/// `sv.protocolflags` inside the glue (as `SvMain_Glue_WriteBatch` does),
/// because `MSGFTE_WriteEntityUpdate` takes `protocolflags` and `pext2` as
/// *parameters* (`sv_send.c:239`) and `MSG_WriteStaticOrBaseLine` is called
/// with a caller-supplied pair (`sv_send.c:963`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SvSendWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub f: c_float,
    pub u: c_uint,
    pub p: *const c_void,
}
