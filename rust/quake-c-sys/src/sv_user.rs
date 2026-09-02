//! `Quake/sv_user_glue.c` declarations (Rust migration Phase 7 M6, T6.4).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the six C-visible objects `Quake/sv_user.c` used to define:
//! `edict_t *sv_player;` (`sv_user.c:26`) and five cvars --
//! `sv_edgefriction` (`:29`, name string `"edgefriction"`, NOT
//! `"sv_edgefriction"`), `sv_idealpitchscale` (`:43`), `sv_altnoclip`
//! (`:44`), `sv_maxspeed` (`:198`) and `sv_accelerate` (`:199`) -- plus the
//! two `Host_Guard` trampolines this port needs (ADR-009).
//!
//! `sv_friction`, `sv_stopspeed` and `sv_analyticphysics_frame` are declared
//! by `Quake/sv_phys_glue.c` (Phase 7 M4) and are read here via
//! `quake_c_sys::sv_phys`, not redeclared -- `sv_user.c` only ever held
//! `extern cvar_t` references to the first two, and a block-scope
//! `extern qboolean` to the third (`sv_user.c:150`, `:289`).
//!
//! ## Finding: `quake-c-sys` cannot depend on `quake-types`
//!
//! `m6-wave.md` says Rust reaches `sv`/`svs`/`cls`/`host_client` "through
//! `extern "C" { static mut sv: Server; }` in `quake-c-sys`". That is not
//! possible as written: `rust/quake-c-sys/Cargo.toml` has no
//! `[dependencies]` section at all, so this crate cannot name
//! `quake_types::host::Server` (or any other ADR-011 mirror). Those four
//! mirror-typed externs are declared instead in
//! `quake-capi/src/sv_user.rs`, which already depends on `quake-types` --
//! preserving the substance of `m6-wave.md` #1 (direct mirror field access,
//! no per-field accessor functions) while deviating from #2's literal
//! crate-placement wording. T6.2/T6.5 (`sv_send.c`/`sv_main.c`) hit the same
//! constraint for the same four symbols; this is reported as a finding, not
//! worked around silently.
//!
//! Engine aggregates that this file's own callers need typed are passed as
//! `c_void` pointers here, matching the M4 (`sv_phys.rs`) convention;
//! `quake-capi`'s `sv_user` module casts them to the ADR-011 mirrors.

use crate::qboolean;
use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

extern "C" {
    /* Quake/sv_user_glue.c data (sv_user.c:26, :29, :43, :44, :198, :199) */

    /// `sv_user.c:26` -- `edict_t *sv_player;`. Real storage moves here from
    /// `Quake/sv_user.c` (Pattern A whole-file swap); `sv_phys_glue.c`'s
    /// existing `SvPhys_Glue_SvPlayer()` accessor keeps working unchanged,
    /// since it only reads this symbol by name.
    pub static mut sv_player: *mut c_void;

    /// `sv_user.c:29`. COMPAT: the registered cvar *name string* is
    /// `"edgefriction"`, not `"sv_edgefriction"` -- preserved verbatim by
    /// `Quake/sv_user_glue.c`.
    pub static mut sv_edgefriction: crate::cvar_t;
    pub static mut sv_idealpitchscale: crate::cvar_t;
    pub static mut sv_altnoclip: crate::cvar_t;
    pub static mut sv_maxspeed: crate::cvar_t;
    pub static mut sv_accelerate: crate::cvar_t;

    /* Quake/sv_user_glue.c guards -- each returns a Host_Guard status */

    /// Wraps the whole `clc_stringcmd` case body (`sv_user.c:577-592`) minus
    /// the `MSG_ReadString` call, which Rust already did (it cannot raise).
    /// Both the `PR_ExecuteProgram`/QC-dispatch branch and the
    /// `Cmd_ExecuteString` branch stay C-to-C inside one guarded frame, so
    /// this signature never needs `q_strncasecmp`, `G_INT`/`OFS_PARM0`,
    /// `PR_SetEngineString`, `pr_global_struct`, `EDICT_TO_PROG`,
    /// `PR_ExecuteProgram`, `Cmd_ExecuteString` or `src_client` on the Rust
    /// side.
    pub fn SvUser_Glue_StringCmd(s: *const c_char) -> c_int;

    /// Wraps `SV_DropClient (crash)` (`sv_user.c:653`, called from
    /// `SV_RunClients`), a confirmed transitive ADR-009 raise site via
    /// `Quake/host.c:590`'s `PR_ExecuteProgram
    /// (pr_global_struct->ClientDisconnect)`.
    pub fn SvUser_Glue_DropClient(crash: qboolean) -> c_int;

    /// Wraps `NET_GetServerMessage ()` (`sv_user.c:628`, called from
    /// `SV_RunClients`). Phase 7 M9: the T6.4 audit filed this with the
    /// non-raising net accessors, which it is not. `net_main.c:768`
    /// dispatches to the driver's `QGetAnyMessage`, and the datagram
    /// implementation (`net_dgrm.c:138`) reaches `SV_ConnectClient`
    /// (`sv_main_glue.c:495`, a `Host_Reraise`) through
    /// `_Datagram_ServerControlPacket` and `SV_DropClient`
    /// (`net_dgrm.c:212`) on a socket timeout. Calling it unguarded let
    /// either longjmp unwind this Rust frame (ADR-009 rule 3).
    ///
    /// `*out` is set to NULL before the guarded call, so it is defined on
    /// every non-zero return.
    pub fn SvUser_Glue_GetServerMessage(out: *mut *mut c_void) -> c_int;

    /* Engine C symbols sv_user.c calls directly; none of these can raise. */

    /// `Quake/common.h` -- resets the read cursor before a client message is
    /// parsed. Never sets `msg_badread`.
    pub fn MSG_BeginReading();
    /// Returns -1 (and sets `msg_badread`) on underflow; never longjmps.
    pub fn MSG_ReadChar() -> c_int;
    pub fn MSG_ReadByte() -> c_int;
    pub fn MSG_ReadShort() -> c_int;
    pub fn MSG_ReadLong() -> c_int;
    pub fn MSG_ReadFloat() -> c_float;
    pub fn MSG_ReadString() -> *const c_char;
    /// `Quake/common.h`: `float MSG_ReadAngle (unsigned int flags);` --
    /// `flags` is `unsigned int`, not `int`.
    pub fn MSG_ReadAngle(flags: c_uint) -> c_float;
    pub fn MSG_ReadAngle16(flags: c_uint) -> c_float;

    /// `Quake/keys.h` -- `keydest_t key_dest;`. `key_game == 0`
    /// (`Quake/keys.h:136-142`).
    pub static mut key_dest: c_int;

    /// `Quake/view.h` -- `float V_CalcRoll (vec3_t angles, vec3_t
    /// velocity);`. Neither parameter is `const` in the real declaration.
    pub fn V_CalcRoll(angles: *mut c_float, velocity: *mut c_float) -> c_float;

    /// `Quake/sv_send.c:504` (Phase 7 M6 T6.2/T6.3 territory) -- reads/writes
    /// `client_t` fields only, no `PR_ExecuteProgram`/`Host_Error`/`Sys_Error`
    /// anywhere in its body; safe to call unguarded.
    pub fn SVFTE_Ack(client: *mut c_void, sequence: c_int);

    /// `Quake/net.h` (`net_main.c`) -- reads a per-socket flag only.
    pub fn NET_QSocketGetProQuakeAngleHack(s: *const c_void) -> qboolean;
}
