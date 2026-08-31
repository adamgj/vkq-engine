//! `Quake/pr_cmds_sv_msg_glue.c` declarations (Rust migration Phase 7 M5,
//! wave 2 Group D: message builtins).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns every `Host_Guard` call site reachable from `quake-capi`'s
//! `progs_builtins_sv_msg` module, plus the leaf `server_t`/`client_t`
//! writers that have no ADR-011 mirror.
//!
//! `World_Glue_NumForEdict` (`Quake/world_glue.c`) already covers this
//! module's one `NUM_FOR_EDICT` use (`G_EDICTNUM`/`WriteDest`'s `MSG_ONE`
//! case) and is declared in `quake_c_sys::world`; `SvPhys_Glue_MaxClients`
//! (`Quake/sv_phys_glue.c`) already covers `svs.maxclients` and is declared
//! in `quake_c_sys::sv_phys`. Neither is redeclared here.

use core::ffi::{c_char, c_double, c_float, c_int, c_longlong, c_ulonglong};

extern "C" {
    /* ---- guarded seams (ADR-009 rule 3) ---- */

    /// `pr_cmds.c:938` `PF_stuffcmd`'s `PR_RunError ("Parm 0 not a client")`,
    /// guarded so the raise can be replayed from `RUST_PF`'s C frame.
    pub fn PRBI_MsgGlue_StuffcmdClientCheck(entnum: c_int) -> c_int;

    /// `progs.h:174` `G_STRING (ofs)`, guarded: `PR_GetString` can
    /// `Host_Error` on a corrupt handle (`pr_edict_arena.c:315`). `*out` is
    /// aliased into the engine's string arena, valid for the life of the
    /// interned string -- not copied.
    pub fn PRBI_MsgGlue_GetString(ofs: c_int, out: *mut *const c_char) -> c_int;

    /// `pr_cmds.c:111` `PF_VarString`, guarded: it resolves `G_STRING` for
    /// every variadic argument, any of which can `Host_Error`, and its own
    /// overflow warnings run inside the guard rather than through
    /// `SvConsole` (`pr_cmds_sv_glue.c`'s guarded-Con_Warning precedent,
    /// `world_glue.c:166-190`). `out` must point at a writable 1024-byte
    /// buffer (`PF_VarString`'s `static char out[1024]`).
    pub fn PRBI_MsgGlue_VarString(first: c_int, out: *mut c_char) -> c_int;

    /* ---- leaves (cannot Host_Error/PR_RunError) ---- */

    /// `pr_cmds.c:944` `Host_ClientCommands ("%s", str)` with the
    /// `host_client` save/restore around it (`pr_cmds.c:942-945`). `entnum`
    /// is 1-based and already range-checked by the caller.
    pub fn PRBI_MsgGlue_ClientCommandsPlain(entnum: c_int, str_: *const c_char);

    /// `host.c:401` `SV_BroadcastPrintf ("%s", str)` -- restated in
    /// `pr_cmds.c:401`.
    pub fn PRBI_MsgGlue_BroadcastPrintfPlain(str_: *const c_char);

    /// `pr_cmds.c:430-431` / `:460-461`: `MSG_WriteChar` + `MSG_WriteString`
    /// on `svs.clients[entnum - 1].message`. `kind` is 0 for `svc_print`
    /// (`PF_sprint`) and 1 for `svc_centerprint` (`PF_centerprint`); `entnum`
    /// is 1-based and already range-checked by the caller.
    pub fn PRBI_MsgGlue_ClientMessageWrite(entnum: c_int, kind: c_int, str_: *const c_char);

    /* ---- extended message writers (pr_ext.c), all sharing WriteDest()'s
    reimplemented dispatch (Rust side resolves `dest`/`entnum`, per the M5
    contract's mandate to reuse PRBI_ERR_WRITEDEST_NOT_CLIENT/BAD_DEST
    rather than guard WriteDest() whole). None of these six can raise. */

    /// `pr_ext.c:2594` `MSG_WriteFloat (WriteDest (), G_FLOAT (OFS_PARM0))`.
    pub fn PRBI_MsgGlue_WriteFloat(dest: c_int, entnum: c_int, f: c_float);

    /// `pr_ext.c:2598` `MSG_WriteDouble (WriteDest (), G_DOUBLE (OFS_PARM0))`.
    pub fn PRBI_MsgGlue_WriteDouble(dest: c_int, entnum: c_int, f: c_double);

    /// `pr_ext.c:2602` `MSG_WriteDouble (WriteDest (), G_INT (OFS_PARM0))`.
    /// COMPAT: `PF_WriteInt` calls `MSG_WriteDouble`, not an int writer --
    /// this reproduces that bug verbatim, letting C's own implicit
    /// int-to-double conversion apply exactly as it does in the original.
    /// `"WriteUInt"`'s table slot (`pr_ext.c:5676`) points at this same
    /// `PF_WriteInt` function pointer, so it shares this entry point too.
    pub fn PRBI_MsgGlue_WriteIntAsDouble(dest: c_int, entnum: c_int, v: c_int);

    /// `pr_ext.c:2606` `MSG_WriteInt64 (WriteDest (), G_INT64 (OFS_PARM0))`.
    pub fn PRBI_MsgGlue_WriteInt64(dest: c_int, entnum: c_int, v: c_longlong);

    /// `pr_ext.c:2610` `MSG_WriteUInt64 (WriteDest (), G_UINT64 (OFS_PARM0))`.
    pub fn PRBI_MsgGlue_WriteUInt64(dest: c_int, entnum: c_int, v: c_ulonglong);

    /// `pr_ext.c:2590` `SZ_Write (WriteDest (), string, strlen (string))`.
    /// `strlen` runs in C so it sees exactly the bytes `string` points at.
    pub fn PRBI_MsgGlue_WriteString2(dest: c_int, entnum: c_int, string: *const c_char);
}
