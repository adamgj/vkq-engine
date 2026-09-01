//! `Quake/sv_main_glue.c` declarations (Rust migration Phase 7 M6, T6.5).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the C-visible objects `Quake/sv_main.c` used to define -- `int
//! sv_protocol` (`sv_main.c:32`), `unsigned int sv_protocol_pext1` (`:34`)
//! and `sv_protocol_pext2` (`:35`), and the two cvars `sv_netsort` (`:37`)
//! and `sv_smoothplatformlerps` (`:38`) -- plus `server_t sv` (`:27`) and
//! `server_static_t svs` (`:28`), which stay C-side storage until T6.6 moves
//! them to Rust (ADR-007). `static char localmodels[MAX_MODELS][8]` (`:30`)
//! has internal linkage, so it becomes a Rust-owned array in
//! `quake-capi/src/sv_main.rs`.
//!
//! The glue also carries the ADR-009 raise topology: every `SvMain_Glue_*`
//! returning `c_int` below is a `Host_Guard` trampoline whose status must be
//! propagated to the caller untouched.
//!
//! ## Finding: `quake-c-sys` cannot depend on `quake-types`
//!
//! The wave contract says Rust reaches `sv`/`svs`/`cls`/`host_client` through
//! `extern "C" { static mut sv: Server; }` "in `quake-c-sys`". That is not
//! possible as written: `rust/quake-c-sys/Cargo.toml` has no
//! `[dependencies]` section at all, so this crate cannot name
//! `quake_types::host::Server` (or any other ADR-011 mirror). Those externs
//! are declared instead in `quake-capi/src/sv_main.rs`, which already depends
//! on `quake-types` -- preserving the substance of the contract (direct
//! mirror field access, no per-field glue accessors) while deviating from its
//! literal crate placement. T6.4 recorded the identical finding for the same
//! symbols (`quake-c-sys/src/sv_user.rs:17-30`); both are reported, not
//! worked around silently.
//!
//! Engine aggregates whose types this file does not need are passed as
//! `c_void` pointers, matching the M4 (`sv_phys.rs`) convention.

use crate::{cvar_t, qboolean, sizebuf_t};
use core::ffi::{c_char, c_float, c_int, c_long, c_uint, c_ulong, c_void};

extern "C" {
    /* ---- Quake/sv_main_glue.c data (sv_main.c:32-38) ---- */

    /// `sv_main.c:32` -- initialised to `PROTOCOL_RMQ`.
    pub static mut sv_protocol: c_int;
    /// `sv_main.c:34` -- initialised to `PEXT1_SUPPORTED_SERVER`.
    pub static mut sv_protocol_pext1: c_uint;
    /// `sv_main.c:35` -- initialised to `PEXT2_SUPPORTED_SERVER`.
    pub static mut sv_protocol_pext2: c_uint;

    /// `sv_main.c:37`. Non-`static` since the T6.1 split because `sv_send.c`
    /// reads it.
    pub static mut sv_netsort: cvar_t;
    /// `sv_main.c:38`. Same.
    pub static mut sv_smoothplatformlerps: cvar_t;

    /* ---- cvars sv_main.c only references (owned elsewhere) ---- */

    /// `Quake/host.c` -- `SV_SpawnServer`'s "no server with no name" test.
    pub static mut hostname: cvar_t;
    /// `Quake/host.c`.
    pub static mut coop: cvar_t;
    /// `Quake/host.c`.
    pub static mut deathmatch: cvar_t;
    /// `Quake/host.c`.
    pub static mut skill: cvar_t;
    /// `Quake/host.c` -- `sv_main.c:40` declares it `extern` explicitly.
    pub static mut nomonsters: cvar_t;
    /// `Quake/pr_edict.c`.
    pub static mut max_edicts: cvar_t;

    /// `Quake/host.c` -- `int current_skill;`.
    pub static mut current_skill: c_int;

    /// `Quake/pr_edict.c` -- `globalvars_t *pr_global_struct;`. Cast to
    /// `quake_types::progs::GlobalVars` by the caller.
    pub static mut pr_global_struct: *mut c_void;

    /* ---- non-raising engine C entry points ---- */

    /// `Quake/common.h` -- case-insensitive bounded compare.
    pub fn q_strncasecmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    /// C runtime.
    pub fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    /// C runtime -- `SV_Protocol_f` needs the end pointer `strtol` writes
    /// back (`sv_main.c:83`), so the parse cannot be reimplemented loosely.
    pub fn strtol(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long;
    /// C runtime -- `SV_Pext_f`'s key/value parse (`sv_main.c:676-677`).
    pub fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong;
    /// C runtime -- `SV_Init`'s `-protocol` argument (`sv_main.c:198`).
    pub fn atoi(nptr: *const c_char) -> c_int;

    /// `Quake/progs.h` -- swaps the ambient qcvm (ADR-008). Never raises.
    pub fn PR_SwitchQCVM(nvm: *mut c_void);

    /// `Quake/screen.h` -- a no-op when no centerprint is pending.
    pub fn SCR_CenterPrintClear();

    /// `Quake/host.c` -- the `cvarcallback_t` `SV_Init` installs on
    /// `sv_gravity`, `sv_friction` and `sv_maxspeed` (`sv_main.c:167-168`,
    /// `:173`).
    pub fn Host_Callback_Notify(var: *mut cvar_t);

    /// `Quake/net.h` -- returns the next pending connection or NULL.
    pub fn NET_CheckNewConnections() -> *mut c_void;
    /// `Quake/net.h`.
    pub fn NET_QSocketGetTrueAddressString(sock: *const c_void) -> *const c_char;
    /// `Quake/net.h`.
    pub fn NET_QSocketSetMSS(sock: *mut c_void, mss: c_int);
    /// `Quake/net.h`.
    pub fn NET_CanSendMessage(sock: *mut c_void) -> qboolean;
    /// `Quake/net.h`.
    pub fn NET_SendMessage(sock: *mut c_void, data: *mut sizebuf_t) -> c_int;

    /* ---- Quake/sv_main_glue.c guards: each returns a Host_Guard status ---- */

    /// `sv_main.c:285` -- `Host_Error ("SV_StartSound: volume = %i", volume)`.
    pub fn SvMain_Glue_ErrorVolume(volume: c_int) -> c_int;
    /// `sv_main.c:293` -- `Host_Error ("SV_StartSound: attenuation = %f", a)`.
    ///
    /// COMPAT: ADR-005 -- this `%f` conversion stays in C. It is the only
    /// floating-point conversion specifier anywhere in `Quake/sv_main.c`;
    /// there are no `%g` and no `%e` sites at all.
    pub fn SvMain_Glue_ErrorAttenuation(attenuation: c_float) -> c_int;
    /// `sv_main.c:296` -- `Host_Error ("SV_StartSound: channel = %i", c)`.
    pub fn SvMain_Glue_ErrorChannel(channel: c_int) -> c_int;

    /// `PR_GetString` (`sv_main.c:581`) -- `Host_Error`s on a handle outside
    /// the string table (`Quake/pr_edict_arena.c`).
    pub fn SvMain_Glue_GetString(handle: c_int, out: *mut *const c_char) -> c_int;

    /// `SVFTE_SetupFrames (client)` (`sv_main.c:614`) -- `Quake/sv_send.c`,
    /// which reaches `Mem_Alloc` and, through the stats path, `PR_GetString`.
    pub fn SvMain_Glue_SetupFrames(client: *mut c_void) -> c_int;

    /// `SV_SendReconnect ()` (`sv_main.c:900`) -- writes every client's
    /// reliable message, so `MSG_Write*` can raise.
    pub fn SvMain_Glue_SendReconnect() -> c_int;

    /// `SV_CreateBaseline ()` (`sv_main.c:1064`) -- reaches `PR_GetString`
    /// and `MSG_Write*` through `MSG_WriteStaticOrBaseLine`.
    pub fn SvMain_Glue_CreateBaseline() -> c_int;

    /// `Host_ClearMemory ()` (`sv_main.c:927`).
    pub fn SvMain_Glue_ClearMemory() -> c_int;

    /// `PR_LoadProgs ("progs.dat", true, PROGHEADER_CRC, pr_ssqcbuiltins,
    /// pr_ssqcnumbuiltins)` (`sv_main.c:951`). The four constant arguments
    /// stay in C so the builtin table never crosses the FFI boundary.
    pub fn SvMain_Glue_LoadProgs() -> c_int;

    /// `Mod_ForName (name, false)` (`sv_main.c:993`) -- `Quake/gl_model.c`
    /// `Host_Error`s on a malformed model.
    pub fn SvMain_Glue_ModForName(name: *const c_char, out: *mut *mut c_void) -> c_int;

    /// `ED_LoadFromFile (qcvm->worldmodel->entities)` (`sv_main.c:1044`).
    pub fn SvMain_Glue_LoadFromFile(data: *const c_char) -> c_int;

    /// `SV_Precache_Model ("progs/player.mdl")` (`sv_main.c:1048`).
    pub fn SvMain_Glue_PrecacheModel(name: *const c_char) -> c_int;

    /// `PR_ExecuteProgram (pr_global_struct->SetNewParms)` (`sv_main.c:748`).
    pub fn SvMain_Glue_CallSetNewParms() -> c_int;

    /// `pr_global_struct->self = EDICT_TO_PROG (ent); PR_ExecuteProgram
    /// (pr_global_struct->SetChangeParms);` (`sv_main.c:858-859`). The `self`
    /// store stays inside the guarded frame because it is part of the same
    /// two-statement C idiom.
    pub fn SvMain_Glue_CallSetChangeParms(ent: *mut c_void) -> c_int;

    /* ---- Quake/sv_main_glue.c non-raising shims ---- */

    /// `sv_main.c:548-550` -- `q_snprintf (message, sizeof (message), "%c\n"
    /// ENGINE_NAME_AND_VER " Server (%i CRC)\n", 2, qcvm->progscrc)`. Kept in
    /// C so the version macro is never duplicated in Rust.
    pub fn SvMain_Glue_ServerinfoPrint(out: *mut c_char, size: usize, crc: c_int);

    /// `sv_main.c:864-865` -- `ddef_t *g = ED_FindGlobal (va ("parm%i",
    /// index)); *out = g ? qcvm->globals[g->ofs] : 0;`. `va` is variadic and
    /// `ED_FindGlobal` cannot raise, so the whole lookup is one unguarded
    /// shim.
    pub fn SvMain_Glue_SpawnParmGlobal(index: c_int, out: *mut c_float);

    /// `sv_main.c:955-964` -- the whole `#if defined(DEBUG) ||
    /// defined(_DEBUG)` per-edict debug-field loop. Kept in C so the
    /// compile-time condition stays where it is authoritative instead of
    /// being mirrored by a Cargo feature.
    pub fn SvMain_Glue_InitDebugEdicts();

    /// `sv_main.c:987` -- `assert (!ent->free)`. `assert` is `NDEBUG`-gated,
    /// so it stays in C for the same reason.
    pub fn SvMain_Glue_AssertEdictNotFree(ent: *mut c_void);

    /* ---- cvar/command registration. Guarded: under -Duse_rust_cvar the
    plain `Cvar_RegisterVariable`, `Cvar_Set`, `Cvar_SetValue` and
    `Cmd_AddCommand` names are themselves `Host_Reraise` wrappers, so calling
    one from a Rust frame would longjmp through it. ---- */

    /// `Cvar_RegisterVariable (var)` (`sv_main.c:164-190`).
    pub fn SvMain_Glue_RegisterVariable(var: *mut cvar_t) -> c_int;

    /// `Cvar_SetCallback (var, Host_Callback_Notify)` (`sv_main.c:167-168`,
    /// `:173`). Pure field stores, so it cannot raise; kept in C only so the
    /// `Host_Callback_Notify` function pointer never crosses the FFI
    /// boundary.
    pub fn SvMain_Glue_SetNotifyCallback(var: *mut cvar_t);

    /// `Cmd_AddCommand ("pext", SV_Pext_f)` and `Cmd_AddCommand
    /// ("sv_protocol", &SV_Protocol_f)` (`sv_main.c:192-193`), in that order.
    /// The two `xcommand_t`s are C wrappers around the Rust cores.
    pub fn SvMain_Glue_AddCommands() -> c_int;

    /// `Cvar_Set (name, value)` (`sv_main.c:894`, `:906`).
    pub fn SvMain_Glue_CvarSet(name: *const c_char, value: *const c_char) -> c_int;

    /// `Cvar_SetValue (name, value)` (`sv_main.c:918`).
    pub fn SvMain_Glue_CvarSetValue(name: *const c_char, value: c_float) -> c_int;

    /* ---- guarded message writing ---- */

    /// Executes `ops[0 .. count]` against `sb` inside one `Host_Guard` frame
    /// and returns its status. Every `MSG_Write*` reaches `SZ_GetSpace`,
    /// which `Host_Error`s when the sizebuf disallows overflow
    /// (`net_msg.c:493`), so no such call may be made straight from Rust.
    pub fn SvMain_Glue_WriteBatch(
        sb: *mut c_void,
        ops: *const SvMainWriteOp,
        count: c_int,
    ) -> c_int;

    /* ---- remaining engine C entry points; none of these can raise ---- */

    /// `Quake/host.c` -- `double realtime;`.
    pub static mut realtime: f64;

    /// `Quake/common.h:400` -- `const char *COM_GetGameNames (qboolean
    /// full);`. Returns a static buffer.
    pub fn COM_GetGameNames(full: qboolean) -> *const c_char;

    /// `Quake/net.h` (`net_main.c`) -- reads a per-socket flag only.
    /// Declared here as well as in `sv_user.rs` so the two ports stay
    /// decoupled; duplicate `extern` declarations in separate modules are
    /// legal and resolve to the same symbol.
    pub fn NET_QSocketGetProQuakeAngleHack(s: *const c_void) -> qboolean;

    /// `Quake/progs.h` -- `int PR_SetEngineString (const char *s);`. Only
    /// ever hands back an existing known-string slot or appends one; it has
    /// no error path.
    pub fn PR_SetEngineString(s: *const c_char) -> c_int;
}

/// One buffered `MSG_Write*` call for `SvMain_Glue_WriteBatch`. Must stay
/// layout-identical to `svmain_write_t` in `Quake/sv_main_glue.c`.
///
/// `kind` selects the writer: 0 `MSG_WriteByte(i)`, 1 `MSG_WriteChar(i)`,
/// 2 `MSG_WriteShort(i)`, 3 `MSG_WriteLong(i)`, 4 `MSG_WriteString(s)`,
/// 5 `MSG_WriteCoord(f, sv.protocolflags)` -- the flags argument is read
/// inside the glue, matching `Quake/pr_cmds_glue.c:295`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SvMainWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub f: c_float,
    pub s: *const c_char,
}
