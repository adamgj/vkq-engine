//! `Quake/net_main.c` declarations for the M9c funnel port (Phase 7, T9.1).
//!
//! ADR-011: engine C symbols are declared only in this crate. M9 already
//! reached `net_main.c`'s globals through `generated.rs` and
//! `net_dgrm_orch.rs`; this module adds only what the fourteen dispatch
//! funnels need on top of those -- the three `net.h`/`net_defs.h` globals with
//! no prior Rust consumer, the `harness.h` net-replay seam, the four
//! `NetMain_*` state accessors `net_main.c` grows under `USE_RUST_NET`, and
//! the five ADR-009 `Host_Guard` trampolines.
//!
//! `Host_Reraise` is deliberately absent (ADR-009 rule 3): only the C funnel
//! wrappers in `net_main.c` call it. Each `NetMain_Glue_*` below returns a
//! `Host_Guard` status (0 = returned normally, 1 = `Host_Error`/
//! `Host_EndGame`, 2 = `screen_error`) which the Rust core propagates upward
//! untouched, and the wrapper re-issues.
//!
//! This crate has no dependencies, so `qsocket_t *` is spelled `*mut c_void`
//! here and cast at the quake-capi call site (the `net_dgrm_orch.rs`
//! `SchedulePollProcedure` idiom).

use crate::qboolean;
use core::ffi::{c_char, c_int, c_void};

extern "C" {
    /* --- Quake/net_main.c globals (no Rust consumer before M9c) --------- */

    /// C: `extern qboolean slistInProgress;` -- net.h:106
    /// (definition net_main.c:60).
    pub static mut slistInProgress: qboolean;

    /// C: `extern qboolean slist_silent;` -- net.h:107
    /// (definition net_main.c:61).
    pub static mut slist_silent: qboolean;

    /// C: `extern int net_numsockets;` -- net_defs.h:183
    /// (definition net_main.c:46). Written once, by `NET_Init`.
    pub static mut net_numsockets: c_int;

    /* --- Quake/harness.h net-replay seam -------------------------------- */

    /// C: `extern qboolean harness_netreplay;` -- harness.h:79
    pub static mut harness_netreplay: qboolean;

    /// C: `void Harness_NetCapture (int direction, int driver, int kind,
    /// const byte *data, int len);` -- harness.h:68
    pub fn Harness_NetCapture(
        direction: c_int,
        driver: c_int,
        kind: c_int,
        data: *const u8,
        len: c_int,
    );

    /// C: `struct qsocket_s *Harness_NetReplayConnect (void);` -- harness.h:81
    pub fn Harness_NetReplayConnect() -> *mut c_void;

    /// C: `qboolean Harness_NetReplayOwns (struct qsocket_s *sock);`
    /// -- harness.h:82
    pub fn Harness_NetReplayOwns(sock: *mut c_void) -> qboolean;

    /// C: `int Harness_NetReplayGetMessage (void);` -- harness.h:83
    pub fn Harness_NetReplayGetMessage() -> c_int;

    /* --- Quake/net_main.c accessors (USE_RUST_NET) ---------------------- */
    /* The net stratum builds with `use_rust_host=false` (the build-rs-chost
     * CI leg), so client/server state is reached only through these funnels,
     * extending the M9 NetMain_SVActive/MaxClients/... set. */

    /// C: `qboolean NetMain_ClsDedicated (void);` -- `cls.state ==
    /// ca_dedicated`, the two tests in `NET_Init`.
    pub fn NetMain_ClsDedicated() -> qboolean;

    /// C: `void NetMain_SetHostClient (int idx);` -- `host_client =
    /// svs.clients + idx`, i.e. exactly what `NET_SendToAll`'s two `for`
    /// headers do. Called once per iteration *and* once with `svs.maxclients`
    /// after each loop, so `host_client` ends where C leaves it.
    pub fn NetMain_SetHostClient(idx: c_int);

    /// C: `qboolean NetMain_HostClientActive (void);` -- `host_client->active`.
    pub fn NetMain_HostClientActive() -> qboolean;

    /// C: `qsocket_t *NetMain_HostClientConnection (void);` --
    /// `host_client->netconnection`.
    pub fn NetMain_HostClientConnection() -> *mut c_void;

    /* --- Quake/net_main.c ADR-009 Host_Guard trampolines ---------------- */

    /// C: `int NetMain_Glue_QGetMessage (qsocket_t *sock, int *out);`
    /// Guards `sfunc.QGetMessage (sock)` (net_main.c:720) -- reaches
    /// `Datagram_GetMessage`, which can `Host_Error` through the dgrm glue.
    pub fn NetMain_Glue_QGetMessage(sock: *mut c_void, out: *mut c_int) -> c_int;

    /// C: `int NetMain_Glue_QGetAnyMessage (qsocket_t **out);`
    /// Guards `net_drivers[net_driverlevel].QGetAnyMessage ()`
    /// (net_main.c:766). Reads `net_driverlevel` ambiently, as the C loop did.
    pub fn NetMain_Glue_QGetAnyMessage(out: *mut *mut c_void) -> c_int;

    /// C: `int NetMain_Glue_DriverConnect (const char *host,
    /// qsocket_t **out);` Guards `dfunc.Connect (host)` (net_main.c:614).
    /// Reads `net_driverlevel` ambiently.
    pub fn NetMain_Glue_DriverConnect(host: *const c_char, out: *mut *mut c_void) -> c_int;

    /// C: `int NetMain_Glue_DriverInit (int *out);`
    /// Guards `net_drivers[net_driverlevel].Init ()` (net_main.c:1021).
    /// Reads `net_driverlevel` ambiently.
    pub fn NetMain_Glue_DriverInit(out: *mut c_int) -> c_int;

    /// C: `int NetMain_Glue_RegisterNetVars (void);`
    /// Guards the three `Cvar_RegisterVariable` and four `Cmd_AddCommand`
    /// calls of `NET_Init` (net_main.c:1011-1018) in one guard, in source
    /// order -- under `-Duse_rust_cvar` both names are themselves
    /// `Host_Reraise` wrappers.
    pub fn NetMain_Glue_RegisterNetVars() -> c_int;

    /* --- shared helpers ------------------------------------------------- */

    /// `Quake/common.c` -- `int q_strcasecmp (const char *s1, const char *s2)`.
    /// Also mirrored by `host_cmd.rs:103`, `lib.rs:88` and
    /// `net_dgrm_orch.rs:193`; a duplicate extern declaration of one symbol is
    /// the established per-module idiom.
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
}
