//! `Quake/net_dgrm_orch_glue.c` declarations (Rust migration Phase 7 M9b, T9.2).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the five C-visible cvar objects `Quake/net_dgrm.c` used to define --
//! `sv_reportheartbeats` (`:44`), `sv_public` (`:45`), `com_protocolname`
//! (`:46`), `net_masters[]` (`:47-55`) and `rcon_password` (`:56`) -- plus the
//! ADR-009 raise trampolines and the handful of formatting seams the port
//! deliberately leaves in C so the wire bytes stay identical.
//!
//! Only symbols the port needs that `generated.rs` does not already carry are
//! declared here. Per the duplication precedent recorded at
//! `quake-c-sys/src/host_cmd.rs:95-98`, shared helpers such as `q_strcasecmp`
//! are redeclared rather than imported cross-module, with byte-identical
//! signatures so `clashing_extern_declarations` stays quiet.
//!
//! `net_landrivers[]` is deliberately absent: it is an incomplete array type
//! in C, so an extern gives `.add(i)` no provenance. Use the existing
//! `NetMain_LanDrivers()` (`generated.rs:921`) base pointer instead.

use crate::{cvar_t, qboolean};
use core::ffi::{c_char, c_int, c_uint, c_void};

/// The one raise code `NetDgrmOrch_Reraise` understands, matching
/// `#define RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW (-2)` in
/// `Quake/net_dgrm_orch_glue.c`. Distinct from `HOST_GUARD_OK` (0) and from
/// `Host_Guard`'s own 1/2 (ADR-009).
pub const RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW: c_int = -2;

extern "C" {

    /* ===== agent A -- lifecycle, stats, rcon ===== */
    /* --- Quake/net_main.c stat counters (net_defs.h decls) ------------ */
    /// C: `extern int messagesSent;` -- net_defs.h:245
    /// (definition `int messagesSent = 0;` at net_main.c:77)
    pub static mut messagesSent: c_int;
    /// C: `extern int unreliableMessagesSent;` -- net_defs.h:247
    /// (definition `int unreliableMessagesSent = 0;` at net_main.c:79)
    pub static mut unreliableMessagesSent: c_int;

    /* --- Quake/net_main.c timeout cvars -------------------------------- */
    /// C: `extern cvar_t net_messagetimeout;` -- net_dgrm.c:56
    /// (definition net_main.c:82, registered net_main.c:1011)
    pub static mut net_messagetimeout: cvar_t;
    /// C: `extern cvar_t net_connecttimeout;` -- net_dgrm.c:57
    /// (definition net_main.c:83, registered net_main.c:1012)
    pub static mut net_connecttimeout: cvar_t;

    /* --- Quake/net_dgrm_orch_glue.c (ADR-009 Host_Guard trampolines) --- */
    /// C: `int NetDgrmOrch_Glue_AddNetStatsCommand (void);`
    /// Guards `Cmd_AddCommand ("net_stats", NET_Stats_f)` (net_dgrm.c:578);
    /// under `-Duse_rust_cvar` the plain `Cmd_AddCommand` name is itself a
    /// `Host_Reraise` wrapper. Returns the `Host_Guard` status, 0 = no raise.
    pub fn NetDgrmOrch_Glue_AddNetStatsCommand() -> c_int;

    /// C: `int NetDgrmOrch_Glue_AddTestCommands (void);`
    /// Guards `Cmd_AddCommand ("test", Test_f)` then
    /// `Cmd_AddCommand ("test2", Test2_f)` (net_dgrm.c:598-599) in one
    /// guard, in that order. Returns the `Host_Guard` status.
    pub fn NetDgrmOrch_Glue_AddTestCommands() -> c_int;

    /// `Quake/net_dgrm_orch_glue.c` -- the C side of net_dgrm.c:213 and
    /// :1055-1063. Scans `svs.clients` for `netconnection == sock`, sets
    /// `host_client` and calls `SV_DropClient (false)`, all inside a
    /// `Host_Guard`: `SV_DropClient` reaches `PR_ExecuteProgram` of
    /// `ClientDisconnect`, so it can raise (ADR-009 rule 3 -- the longjmp must
    /// not cross a Rust frame). `close_first` selects :1059's
    /// `NET_Close`-before-drop, which sits inside the match on both sides.
    ///
    /// `sock` is a `qsocket_t *`; this crate has no dependencies, so the
    /// mirror type cannot be named here.
    pub fn NetDgrmOrch_Glue_DropClient(sock: *mut c_void, close_first: qboolean) -> c_int;

    /* --- Quake/net_dgrm_orch_glue.c (Con_Redirect callback) ------------ */
    /// C: `void Datagram_Rcon_Flush (const char *text);` -- net_dgrm.c:671.
    /// Declared in no header; agent B takes its address at net_dgrm.c:936
    /// for `Con_Redirect`. The glue keeps the exact C signature and
    /// calling convention so the redirect machinery is untouched.
    pub fn Datagram_Rcon_Flush(text: *const c_char);

    /* ===== agent B -- server control packet ===== */
    /* ---------------------------------------------------------------
     * C-owned cvar objects (contract: these five stay in
     * `Quake/net_dgrm_orch_glue.c`; `cvar_s` is a bindgen-known layout,
     * `generated.rs:376-385`, so the port reads `.value` / `.string`
     * directly rather than through an accessor).
     * --------------------------------------------------------------- */

    /// `net_dgrm.c:44` -- `cvar_t sv_reportheartbeats`.
    pub static mut sv_reportheartbeats: cvar_t;

    /// `net_dgrm.c:45` -- `cvar_t sv_public`.
    pub static mut sv_public: cvar_t;

    /// `net_dgrm.c:46` -- `cvar_t com_protocolname`.
    pub static mut com_protocolname: cvar_t;

    /// `net_dgrm.c:47-55` -- `cvar_t net_masters[]`: seven master-server
    /// cvars plus a `{NULL}` sentinel element. The count is baked into
    /// `NET_MASTERS_COUNT` on the Rust side; keep the two in sync.
    pub static mut net_masters: [cvar_t; 8];

    /// `net_dgrm.c:60` -- `cvar_t rcon_password`.
    pub static mut rcon_password: cvar_t;

    /// `cmd.h:139` -- `void Cmd_TokenizeString (const char *text);`.
    /// Tokenises into `cmd_argc`/`cmd_argv`; cannot raise.
    pub fn Cmd_TokenizeString(text: *const c_char);

    /// `cvar.h:144` -- `cvar_t *Cvar_FindVarAfter (const char *prev_name,
    /// unsigned int with_flags);`. Pure list walk; cannot raise.
    pub fn Cvar_FindVarAfter(prev_name: *const c_char, with_flags: c_uint) -> *mut cvar_t;

    /// `console.h` -- `void Con_Redirect (void (*flush) (const char *));`.
    /// Flushes any *existing* redirect through the old callback, clears the
    /// buffer, then installs `flush` (`console.c:1528-1534`). Cannot raise.
    pub fn Con_Redirect(flush: Option<unsafe extern "C" fn(text: *const c_char)>);

    /* ---------------------------------------------------------------
     * `Quake/net_dgrm_orch_glue.c` -- Host_Guard trampolines and the two
     * preprocessor-only values. All return a `Host_Guard` status
     * (`quakedef.h`: 0 OK / 1 ABORTSERVER / 2 SCREEN_ERROR) which the
     * caller must propagate untouched to its C vtable wrapper (ADR-009).
     * --------------------------------------------------------------- */

    /// `net_dgrm.c:936` -- `Cmd_ExecuteString (cmd, src_command)`, the rcon
    /// command dispatch. `src_command` is supplied by the trampoline so the
    /// port never has to name the `cmd_source_t` enumerator.
    pub fn NetDgrmOrch_Glue_ExecuteString(text: *const c_char) -> c_int;

    /// `net_dgrm.c:1121` -- `SV_ConnectClient (plnum)`. Routed through the
    /// glue rather than calling `quake_rs_sv_connect_client` directly: the
    /// `build-rs-chost` CI leg has `use_rust_net` on with `use_rust_host`
    /// off, where the Rust export does not exist.
    pub fn NetDgrmOrch_Glue_ConnectClient(clientnum: c_int) -> c_int;

    /// `net_dgrm.c:744` -- the assembled literal `"\ver\\"
    /// ENGINE_NAME_AND_VER`. `ENGINE_NAME_AND_VER` is a build-time
    /// preprocessor concatenation, so the finished string is exported from
    /// C the same way `host_cmd.c`'s version line is
    /// (`quake-c-sys/src/host_cmd.rs:187`'s `HostCmd_EngineVersionLine`,
    /// whose data-export shape this copies). Never NULL.
    pub static NetDgrmOrch_GetInfoVerField: *const c_char;

    /* ---------------------------------------------------------------
     * Agent A's range (net_dgrm.c:65-687) -- ASSUMED, not declared by me.
     * Listed here only so the merge notices if A's shape differs.
     *
     *   /// `net_dgrm.c:672` -- `static void Datagram_Rcon_Flush (const
     *   /// char *text)`, needed by value as the `Con_Redirect` callback.
     *   /// If A keeps it a private Rust `unsafe extern "C" fn`, drop this
     *   /// extern and pass `Some(Datagram_Rcon_Flush)` directly.
     *   pub fn Datagram_Rcon_Flush (text: *const c_char);
     * --------------------------------------------------------------- */

    /* ===== agent C1 -- server discovery ===== */
    // -- message reader (implemented in Rust by quake-capi/src/net.rs, but
    //    reached as ordinary C-ABI symbols so msg_readcount / msg_badread /
    //    harness_badread_count stay exactly in step with the C build).
    //    Duplicates of quake-c-sys/src/sv_user.rs:93-100.

    /// C: `void MSG_BeginReading (void)` (`common.h`)
    pub fn MSG_BeginReading();
    /// C: `int MSG_ReadByte (void)` (`common.h`)
    pub fn MSG_ReadByte() -> c_int;
    /// C: `int MSG_ReadLong (void)` (`common.h`)
    pub fn MSG_ReadLong() -> c_int;
    /// C: `const char *MSG_ReadString (void)` (`common.h`)
    pub fn MSG_ReadString() -> *const c_char;

    // -- string / conversion leaves ----------------------------------------

    /// C: `size_t q_strlcpy (char *dst, const char *src, size_t size)`
    /// (`q_stdinc.h`). Duplicate of quake-c-sys/src/host_cmd.rs:102.
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    /// C: `size_t strlen (const char *)` (`<string.h>`).
    /// Duplicate of quake-c-sys/src/sv_send.rs:73.
    pub fn strlen(s: *const c_char) -> usize;
    /// C: `char *strcpy (char *dst, const char *src)` (`<string.h>`).
    /// Duplicate of quake-c-sys/src/cl_main.rs:300.
    pub fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    /// C: `char *strcat (char *dst, const char *src)` (`<string.h>`).
    /// New: `net_dgrm.c:1406` / `:1491` are the only `strcat` call sites in
    /// the ported set, and they are the deliberate overflow documented in
    /// `star_prefix_name`.
    pub fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    /// C: `int atoi (const char *)` (`<stdlib.h>`).
    /// Duplicate of quake-c-sys/src/host.rs:289.
    pub fn atoi(s: *const c_char) -> c_int;

    /// `Quake/common.c` -- `int q_strcasecmp (const char *s1, const char *s2)`.
    /// Also mirrored by `host_cmd.rs:103` and `lib.rs:88`; a duplicate extern
    /// declaration of one symbol is the established per-module idiom.
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;

    /// `net_defs.h:215` -- `extern const int net_numlandrivers;`.
    pub static net_numlandrivers: c_int;

    // -- net_main.c globals -------------------------------------------------
    /// C: `enum slistScope_e slist_scope` (`net.h:108`, defined at
    /// `net_main.c:62`). Declared as the underlying `int`: the enum has no
    /// negative members and fits in `int` on every supported target.
    /// `static mut` because net_main.c writes it (`:536`).
    pub static mut slist_scope: c_int;

    // -- net_dgrm_orch_glue.c seams ----------------------------------------

    /// C: `const char *NetDgrmOrch_Glue_MasterString (size_t m)`
    /// (`net_dgrm_orch_glue.c`) -- `net_masters[m].string`, the cvar array
    /// that stays C-side per the storage split. Returns NULL past the last
    /// element, which is how `net_dgrm.c:1291` terminates its loop.
    pub fn NetDgrmOrch_Glue_MasterString(m: usize) -> *const c_char;
    /// C: `const char *NetDgrmOrch_Glue_ProtocolName (void)`
    /// (`net_dgrm_orch_glue.c`) -- `com_protocolname.string`
    /// (`net_dgrm.c:1298`).
    pub fn NetDgrmOrch_Glue_ProtocolName() -> *const c_char;
    /// C: `const char *NetDgrmOrch_Glue_MasterQuery (int ipv6,
    /// const char *token, unsigned int protover)`
    /// (`net_dgrm_orch_glue.c`) -- the two `va (...)` master queries at
    /// `net_dgrm.c:1301-1304`, kept in C so the format strings and `va`'s
    /// VA_BUFFERLEN truncation stay byte-identical.
    pub fn NetDgrmOrch_Glue_MasterQuery(
        ipv6: c_int,
        token: *const c_char,
        protover: c_uint,
    ) -> *const c_char;
    /// C: `int NetDgrmOrch_Glue_AfInet (void)` (`net_dgrm_orch_glue.c`) --
    /// the platform's `AF_INET`. `quake_net::udp`'s copy is `#[cfg(unix)]`
    /// and so is unreachable on the Windows leg.
    pub fn NetDgrmOrch_Glue_AfInet() -> c_int;
    /// C: `int NetDgrmOrch_Glue_AfInet6 (void)` (`net_dgrm_orch_glue.c`) --
    /// see `NetDgrmOrch_Glue_AfInet`.
    pub fn NetDgrmOrch_Glue_AfInet6() -> c_int;

    /* ===== agent C2 -- connect handshake ===== */

    /* --------------------------------------------------------------------
     * Menu return-path state. `Quake/net_dgrm.c:61-62` reaches these with
     * file-scope `extern` declarations; the real storage is in
     * `Quake/menu.c:92-93` and stays there (it is menu state, not net state).
     */

    /// `Quake/menu.c:92` -- `qboolean m_return_onerror;`. Set by the menu
    /// (`menu.c:3873`, `:4530`) before a connect attempt; `_Datagram_Connect`
    /// clears it on success (`net_dgrm.c:1803`) and consumes it on the
    /// failure path (`:1810-1815`).
    pub static mut m_return_onerror: qboolean;

    /// `Quake/menu.c:93` -- `char m_return_reason[32];`. The 32-byte bound is
    /// behaviour: `net_dgrm.c:1718` writes an attacker-supplied
    /// `MSG_ReadString` result (up to 2047 bytes) through
    /// `q_strlcpy (..., sizeof (m_return_reason))`, i.e. 31 bytes + NUL.
    /// `menu.c:3790` / `:4503` print it.
    pub static mut m_return_reason: [c_char; 32];

    /// `Quake/menu.h:53` -- `extern enum m_state_e m_state;`. Declared
    /// `c_int`: `enum m_state_e` (`menu.h:26-51`) has 25 enumerators, all
    /// non-negative and small, so the C compiler gives it `int` as its
    /// compatible type. Same convention as the existing `key_dest`
    /// declarations in `sv_user.rs`, `host_cmd.rs` and `cl_demo.rs`.
    pub static mut m_state: c_int;

    /// `Quake/menu.h:54` -- `extern enum m_state_e m_return_state;`. See
    /// `m_state` for the `c_int` choice.
    pub static mut m_return_state: c_int;

    /// `Quake/keys.h:144` -- `keydest_t key_dest;`. `key_menu == 3`
    /// (`keys.h:136-142`). SHARED with `sv_user.rs` / `host_cmd.rs` /
    /// `cl_demo.rs`, which already declare it in their own modules; this is
    /// the fourth such per-module declaration, matching that precedent.
    pub static mut key_dest: c_int;

    /* --------------------------------------------------------------------
     * Quake/net_dgrm_orch_glue.c ADR-009 trampolines. SHARED: agent B needs
     * DropClient / ConnectClient / ExecuteString; C2 needs only UpdateScreen.
     */

    /// Wraps `SCR_UpdateScreen (false)` in a `Host_Guard`
    /// (`net_dgrm.c:1589`, `:1632`, `:1693` -- all three inside
    /// `_Datagram_Connect`). Returns `HOST_GUARD_OK` or the caught jump;
    /// `Datagram_Connect`'s C wrapper re-raises it.
    pub fn NetDgrmOrch_Glue_UpdateScreen() -> c_int;

    /* ===== agent D -- test / test2 ===== */
    /// `Quake/net_defs.h:280` -- `void SchedulePollProcedure (PollProcedure
    /// *pp, double timeOffset);`. Still a plain C function: the M9 ADR-009
    /// audit (see rust/quake-capi/src/net_main.rs's module doc comment)
    /// left `NET_Poll`/`SchedulePollProcedure` (both in net_main.c, not
    /// net_dgrm.c) as required C frames, and nothing in this M9b contract
    /// ports net_main.c. Takes no raise path (it just links a
    /// `PollProcedure` into the scheduler's list and stores a deadline), so
    /// no `Host_Guard` trampoline is needed -- called as a plain extern,
    /// same posture as `Sys_Error`.
    /// `pp` is a `PollProcedure *`. This crate has no dependencies (not even
    /// `quake-types`), so the mirror type cannot be named here; callers pass
    /// `(&raw mut PROC).cast()` from a `quake_types::net::PollProcedure`.
    pub fn SchedulePollProcedure(pp: *mut c_void, time_offset: f64);
}
