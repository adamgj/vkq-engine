//! `Quake/host_cmd_glue.c` declarations (Rust migration Phase 7 M8, T8.3).
//!
//! ADR-011: engine C symbols are declared only in this crate. `host_cmd.c`
//! defined seven C-visible data objects; all seven keep C storage in
//! `Quake/host_cmd_glue.c`, so no ADR-007 row opens or closes at T8.3. Two of
//! them already have a Rust declaration elsewhere and are *not* repeated here
//! -- `current_skill` in [`crate::sv_main`] and `noclip_anglehack` in
//! [`crate::view`]; use those.
//!
//! Each `HostCmd_Glue_*` returns a `Host_Guard` status (0 = returned normally,
//! 1 = `Host_Error`/`Host_EndGame`, 2 = `screen_error`) which the Rust core
//! propagates upward untouched. `Host_Reraise` is deliberately absent: only
//! the glue calls it (ADR-009 rule 3).
//!
//! The `atomics.h` accessors `host_cmd.c` used are `static inline` with
//! compiler-specific barriers, so they are reached through
//! `HostCmd_Glue_Atomic*` seams rather than re-derived with Rust orderings.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void};

use crate::{cmd_source_t, cvar_t, qboolean, MAX_OSPATH};

// ==== CHUNK A mirrors (host_cmd.c:24-898) ====
//
// Layout mirrors for the three `common.h` filesystem structs `ExtraMaps_Init`,
// `FileList_Init` and `Modlist_Init` walk. None of them has a `quake_types`
// mirror, and all three carry `MAX_OSPATH`/`MAX_QPATH` arrays whose length is
// platform-dependent, so they are hand-written here for the same reason
// `generated.rs:26` hand-writes `findfile_s`.

/// `q_types.h:240` -- `MAX_QPATH`.
pub const MAX_QPATH: usize = 64;
/// `common.h:371` -- `MAX_BASEDIRS`.
pub const MAX_BASEDIRS: usize = 4;

/// `common.h:340-344` -- `packfile_t`.
#[repr(C)]
pub struct packfile_t {
    pub name: [c_char; MAX_QPATH],
    pub filepos: c_int,
    pub filelen: c_int,
}

/// `common.h:346-352` -- `pack_t`.
#[repr(C)]
pub struct pack_t {
    pub filename: [c_char; MAX_OSPATH],
    pub handle: c_int,
    pub numfiles: c_int,
    pub files: *mut packfile_t,
}

/// `common.h:354-363` -- `searchpath_t`.
#[repr(C)]
pub struct searchpath_t {
    pub path_id: c_uint,
    pub filename: [c_char; MAX_OSPATH],
    pub pack: *mut pack_t,
    pub dir: [c_char; MAX_QPATH],
    pub next: *mut searchpath_t,
}

extern "C" {
    // -- data ---------------------------------------------------------------
    /// `quakedef.h:418-421` -- the four `filelist_item_t *` heads. Declared as
    /// `*mut c_void` here; `quake-capi` casts to its `FileListItem` mirror.
    pub static mut extralevels: *mut c_void;
    pub static mut extralevels_sorted: *mut *mut c_void;
    pub static mut modlist: *mut c_void;
    pub static mut demolist: *mut c_void;
    pub static mut savelist: *mut c_void;

    // -- atomics (host_cmd.c's three `Atomic_*` shapes) ----------------------
    /// `atomics.h:53` / `:192` -- acquire-ish load of a `atomic_uint32_t`.
    pub fn HostCmd_Glue_AtomicLoadU32(atomic: *mut c_void) -> c_uint;
    /// `atomics.h:59` / `:197` -- release-ish store of a `atomic_uint32_t`.
    pub fn HostCmd_Glue_AtomicStoreU32(atomic: *mut c_void, desired: c_uint);
    /// `atomics.h:161` / `:276` -- load of an `atomic_ptr_t`.
    pub fn HostCmd_Glue_AtomicLoadPtr(atomic: *mut c_void) -> *mut c_void;
    /// `atomics.h:167` / `:281` -- store to an `atomic_ptr_t`.
    pub fn HostCmd_Glue_AtomicStorePtr(atomic: *mut c_void, desired: *mut c_void);

    // -- the map-description parsing thread (ADR-016) ------------------------
    /// `host_cmd.c:461` -- `QThread_Create (ExtraMaps_ParseDescriptions, ...)`.
    /// The glue owns the handle and the cancel flag; `fn` is the Rust worker.
    pub fn HostCmd_Glue_StartParsingThread(func: unsafe extern "C" fn(*mut c_void) -> c_int);
    /// `host_cmd.c:398-406` -- join the worker and clear the cancel flag.
    pub fn HostCmd_Glue_WaitForParsingThread();
    /// `host_cmd.c:473` -- raise the cancel flag before a join.
    pub fn HostCmd_Glue_SetCancelParsing(value: c_uint);
    /// `host_cmd.c:377` -- the worker's poll of that flag.
    pub fn HostCmd_Glue_GetCancelParsing() -> c_uint;

    // -- shared plain C calls ------------------------------------------------
    // Used by more than one chunk. Per the established per-module duplication
    // precedent (`cl_demo.rs`, `cl_main.rs`, `cl_parse.rs` each carry their own
    // copy), these are declared here rather than imported cross-module; the
    // signatures must agree with those copies exactly or
    // `clashing_extern_declarations` fires.
    pub fn va(format: *const c_char, ...) -> *mut c_char;
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    pub fn strchr(s: *const c_char, c: c_int) -> *mut c_char;
    pub fn strrchr(s: *const c_char, c: c_int) -> *mut c_char;
    pub fn sprintf(str_: *mut c_char, fmt: *const c_char, ...) -> c_int;
    pub fn atof(s: *const c_char) -> c_double;
    /// `net.h:59` -- plain getter, no error path.
    pub fn NET_QSocketGetTime(sock: *const c_void) -> f64;
    /// `quakedef.h` -- `cmd_source_t cmd_source`.
    pub static mut cmd_source: cmd_source_t;

    // =======================================================================
    // Guarded seams (ADR-009 rule 3). Appended per chunk during T8.3; keep the
    // chunk banners so the merge stays reviewable.
    // =======================================================================

    // ==== CHUNK A (host_cmd.c:24-898) ====

    // -- plain engine symbols the chunk needs that `generated.rs` lacks -----
    /// `common.h:365-366` -- the searchpath chain, and the head of the id1
    /// group (`Modlist_Init` compares `path_id` against it).
    pub static mut com_searchpaths: *mut searchpath_t;
    pub static mut com_base_searchpaths: *mut searchpath_t;
    /// `common.h:372-374` -- every content root, in mount order.
    pub static mut com_basedirs: [[c_char; MAX_OSPATH]; MAX_BASEDIRS];
    pub static mut com_numbasedirs: c_int;

    /// `gl_model.h:756` -- the worldspawn "message" reader. Called from the
    /// `ExtraMaps_ParseDescriptions` worker, so it must stay a plain call
    /// (ADR-016: no `Host_Guard` frame on that thread).
    pub fn Mod_LoadMapDescription(
        desc: *mut c_char,
        maxchars: usize,
        map: *const c_char,
    ) -> qboolean;

    /// `common.h:307` -- copies `in` into `out` with `substr` colour-tinted.
    pub fn COM_TintSubstring(
        in_: *const c_char,
        substr: *const c_char,
        out: *mut c_char,
        outsize: usize,
    ) -> *mut c_char;

    /// `common_fs.c:717` -- rejects mod dir names containing path separators.
    pub fn COM_ModForbiddenChars(p: *const c_char) -> qboolean;

    /// `common.h:322` -- the unformatted localization string for a `$key`,
    /// or NULL. `Modlist_GetFullName` resolves rerelease `$m_*` keys with it.
    pub fn LOC_GetRawString(key: *const c_char) -> *const c_char;

    /// `host_cmd.c:56` -- `M_Menu_Quit_f ()`, which reaches `Cbuf_AddText`.
    pub fn HostCmd_Glue_M_Menu_Quit_f() -> c_int;
    /// `host_cmd.c:59` -- `CL_Disconnect ()`.
    pub fn HostCmd_Glue_CL_Disconnect() -> c_int;
    /// `host_cmd.c:60` -- `Host_ShutdownServer (false)`. `qboolean` crosses as
    /// the `int` the `HOST_GUARD_INT` shape uses (`SCR_UpdateScreen`
    /// precedent, `host_glue.c:531`); the seam narrows it back.
    pub fn HostCmd_Glue_Host_ShutdownServer(crash: c_int) -> c_int;
    /// `host_cmd.c:62` -- `Sys_Quit ()`, which runs `Host_Shutdown` (a
    /// re-raising thunk under `-Duse_rust_host`) before `exit (0)`.
    pub fn HostCmd_Glue_Sys_Quit() -> c_int;
    /// `host_cmd.c:795` -- `COM_LoadFile ("mapdb.json", &path_id)`. Both
    /// results leave through out-parameters so the `int` return stays the
    /// guard status (`host_glue.c:683` precedent).
    pub fn HostCmd_Glue_ComLoadFile(
        path: *const c_char,
        path_id: *mut c_uint,
        out: *mut *mut c_void,
    ) -> c_int;
    // ==== end CHUNK A ====

    // ==== CHUNK B (host_cmd.c:899-1509) ====

    /// `keys.h` -- `keydest_t key_dest`. Declared `c_int` to agree with the
    /// existing `cl_demo.rs:123` / `sv_user.rs:94` copies.
    pub static mut key_dest: c_int;
    /// New C-visible data (an eighth beyond this module's original seven, see
    /// the module doc's ADR-007 note): `"version: " ENGINE_NAME_AND_VER "\n"`,
    /// pre-expanded and stored as a `const char *const` in the glue, because
    /// `ENGINE_NAME_AND_VER` (`quakever.h:59-61`) depends on a build-date macro
    /// (`QSS_DATE`) that cannot be reproduced as a fixed Rust string literal.
    /// Used verbatim (no format specifiers) by `Host_Status_f`
    /// (`host_cmd.c:931`).
    pub static HostCmd_EngineVersionLine: *const c_char;

    /// `cvar.c`/`cvar.rs:512` (Rust, under `-Duse_rust_cvar`) -- declared plain
    /// here (not called via `crate::cvar`) so this chunk links correctly under
    /// `--features host` alone, without also requiring the `cvar` feature.
    pub fn Cvar_VariableString(var_name: *const c_char) -> *const c_char;
    /// `net.h:74` -- `int NET_ListAddresses (qhostaddr_t *addresses, int
    /// maxaddresses);`. `addresses` is untyped here; `quake-capi` passes a
    /// `[[c_char; NET_NAMELEN]; N]` array cast to `*mut c_void`.
    pub fn NET_ListAddresses(addresses: *mut c_void, maxaddresses: c_int) -> c_int;
    /// Parameter type matches the pre-existing `sv_main.rs`/`sv_send.rs`
    /// declarations of the same real symbols (`*const c_void`).
    pub fn NET_QSocketGetTrueAddressString(sock: *const c_void) -> *const c_char;
    pub fn NET_QSocketGetMaskedAddressString(sock: *const c_void) -> *const c_char;
    /// `world.c` -- `void SV_LinkEdict (edict_t *ent, qboolean
    /// touch_triggers);`. `qboolean` is one byte: `bool`.
    pub fn SV_LinkEdict(ent: *mut c_void, touch_triggers: qboolean);
    /// `in_sdl.c` -- re-centers/grabs input; does not reach `Host_Error`.
    pub fn IN_Activate();

    /// `host_cmd.c:1341` -- `Cmd_ExecuteString ("connect local", src_command)`
    /// in `Host_Map_f`. `Cmd_ExecuteString` cannot itself longjmp, but the
    /// command handler it dispatches to transitively can, so the recursive
    /// dispatch is guarded like any other call reaching arbitrary engine code.
    /// The `qboolean` result (was a command found) is discarded, matching
    /// `host_cmd.c`'s own ignored return value.
    pub fn HostCmd_Glue_CmdExecuteString(text: *const c_char, src: c_uint) -> c_int;
    /// `host_cmd.c:1497` -- `CL_EstablishConnection (name)`, called from
    /// `Host_Connect_f`; also `host_cmd.c:2148`'s `("local")` (chunk C).
    pub fn HostCmd_Glue_CLEstablishConnection(name: *const c_char) -> c_int;
    /// `host_cmd.c:1414` -- `Host_Error ("cannot find map %s", level)` in
    /// `Host_Changelevel_f`.
    pub fn HostCmd_Glue_ErrorCannotFindMap(name: *const c_char) -> c_int;
    /// `host_cmd.c:1425` -- `Host_Error ("cannot run map %s", level)` in
    /// `Host_Changelevel_f` (the "also issue an error if spawn failed" O.S.
    /// patch).
    pub fn HostCmd_Glue_ErrorCannotRunMap(name: *const c_char) -> c_int;
    /// `host_cmd.c:1457` -- `Host_Error ("cannot restart map %s", mapname)` in
    /// `Host_Restart_f`.
    pub fn HostCmd_Glue_ErrorCannotRestartMap(name: *const c_char) -> c_int;
    // ==== end CHUNK B ====

    // ==== CHUNK C (host_cmd.c:1510-2156) ====

    // -- plain calls (no callee below can reach Host_Error/Host_EndGame) ----

    /// `common.h:443` -- `host_cmd.c:1531`. `common.c:1275`: no raise.
    pub fn COM_SanitizeDescriptionString(
        dst: *mut c_char,
        dstsize: usize,
        src: *const c_char,
        remove_color: qboolean,
    ) -> usize;

    /// `common.h:420` -- `host_cmd.c:1874`.
    pub fn COM_ParseIntNewline(buffer: *const c_char, value: *mut c_int) -> *const c_char;
    /// `common.h:424` -- `host_cmd.c:1884`, `:1888`, `:1894`.
    pub fn COM_ParseFloatNewline(buffer: *const c_char, value: *mut c_float) -> *const c_char;
    /// `common.h:428` -- `host_cmd.c:1882`, `:1892`, `:1946`.
    pub fn COM_ParseStringNewline(buffer: *const c_char) -> *const c_char;

    /// `glquake.h:709` -- `host_cmd.c:1689`. COMPAT (ADR-005): this is where
    /// the savegame's `%g` fog numbers are formatted; the Rust side only ever
    /// passes the result through `%s`.
    pub fn Fog_GetFogCommand(always: qboolean) -> *const c_char;
    /// `glquake.h:814` -- `host_cmd.c:1693`. Same ADR-005 note as above.
    pub fn Sky_GetSkyCommand(always: qboolean) -> *const c_char;

    /// `glquake.h:704` -- `host_cmd.c:2044`.
    pub fn Fog_Update(density: c_float, red: c_float, green: c_float, blue: c_float, time: c_float);
    /// `glquake.h:708` -- `host_cmd.c:2128`.
    pub fn Fog_ResetFade();
    /// `glquake.h:815` -- `host_cmd.c:2054`.
    pub fn Sky_SetSkyfog(value: c_float);

    /// `screen.h:38` -- `host_cmd.c:1862`, `:1928`, `:2152`.
    pub fn SCR_EndLoadingPlaque();
    /// `q_sound.h:98` -- `host_cmd.c:1938`.
    pub fn S_StopAllSounds(clear: qboolean, keep_statics: qboolean);
    /// `progs.h:121` -- `host_cmd.c:1951`.
    pub fn PR_ClearEdictStrings();
    /// `view.h:32` -- `host_cmd.c:2127`.
    pub fn V_ResetBlend();
    /// `glquake.h:735` -- `host_cmd.c:2129`.
    pub fn R_ClearParticles();
    /// `glquake.h:129` -- `host_cmd.c:2131`. `quakedef.h:38` defines
    /// `PSET_SCRIPT` unconditionally, so the C `#ifdef` is always taken.
    pub fn PScript_ClearParticles(load: qboolean);

    // -- guarded seams -------------------------------------------------------

    /// `EDICT_NUM` (`host_cmd.c:1652`, `:1779`, `:2075`, `:2112`). The macro
    /// `Host_Error`s on an out-of-range index (`pr_edict.c`), so it is guarded
    /// and returns the edict through `*out`.
    pub fn HostCmd_Glue_EdictNum(n: c_int, out: *mut *mut c_void) -> c_int;
    /// `ED_WriteGlobals (f)` (`host_cmd.c:1649`).
    pub fn HostCmd_Glue_EDWriteGlobals(f: *mut c_void) -> c_int;
    /// `ED_Write (f, ed)` (`host_cmd.c:1652`).
    pub fn HostCmd_Glue_EDWrite(f: *mut c_void, ed: *mut c_void) -> c_int;
    /// `ED_CheckFreeList ()` (`host_cmd.c:1703`).
    pub fn HostCmd_Glue_EDCheckFreeList() -> c_int;
    /// `SaveList_Rebuild ()` (`host_cmd.c:1708`) -- chunk A owns the body; the
    /// glue's C wrapper is what is guarded here.
    pub fn HostCmd_Glue_SaveListRebuild() -> c_int;

    /// Replays a run of `MSG_Write*` calls against one `sizebuf_t` inside a
    /// single `Host_Guard` frame (`host_cmd.c:1723-1789`). Every writer reaches
    /// `SZ_GetSpace`, which `Host_Error`s on overflow (`net_msg.c:488`).
    pub fn HostCmd_Glue_WriteBatch(
        sb: *mut c_void,
        ops: *const HostCmdWriteOp,
        count: c_int,
    ) -> c_int;
    /// `SV_WriteClientdataToMessage (c, &c->message)` (`host_cmd.c:1789`).
    pub fn HostCmd_Glue_SVWriteClientdataToMessage(client: *mut c_void, sb: *mut c_void) -> c_int;

    /// `Cvar_SetValue ("skill", ...)` (`host_cmd.c:1890`) -- a cvar callback
    /// can `Host_Error`.
    pub fn HostCmd_Glue_CvarSetValue(name: *const c_char, value: c_float) -> c_int;
    /// `Cvar_SetValueQuick (&nomonsters, 0.f)` (`host_cmd.c:1832`).
    pub fn HostCmd_Glue_CvarSetValueQuick(var: *mut cvar_t, value: c_float) -> c_int;

    /// `CL_Disconnect_f ()` (`host_cmd.c:1914`, `:3123`).
    pub fn HostCmd_Glue_CLDisconnect_f() -> c_int;
    /// `CL_Stop_f ()` (`host_cmd.c:1916`).
    pub fn HostCmd_Glue_CLStop_f() -> c_int;
    /// `SV_SpawnServer (mapname)` (`host_cmd.c:1921`).
    pub fn HostCmd_Glue_SVSpawnServer(name: *const c_char) -> c_int;
    /// `CL_Resume_Record (fastload)` (`host_cmd.c:1941`).
    pub fn HostCmd_Glue_CLResumeRecord(fastload: qboolean) -> c_int;

    /// `Mod_ForName (name, crash)` (`host_cmd.c:1991`, `:2907`). `crash=true`
    /// `Host_Error`s from `gl_model.c:531`; the model comes back in `*out`.
    pub fn HostCmd_Glue_ModForName(
        name: *const c_char,
        crash: qboolean,
        out: *mut *mut c_void,
    ) -> c_int;
    /// `Sky_LoadSkyBox (com_token)` (`host_cmd.c:2049`) -- guarded
    /// conservatively: it reaches the image loaders.
    pub fn HostCmd_Glue_SkyLoadSkyBox(name: *const c_char) -> c_int;

    /// `ED_ParseGlobals (data)` (`host_cmd.c:2071`); the advanced cursor comes
    /// back in `*out`.
    pub fn HostCmd_Glue_EDParseGlobals(data: *const c_char, out: *mut *const c_char) -> c_int;
    /// `ED_ParseEdict (data, ent)` (`host_cmd.c:2097`); cursor in `*out`.
    pub fn HostCmd_Glue_EDParseEdict(
        data: *const c_char,
        ent: *mut c_void,
        out: *mut *const c_char,
    ) -> c_int;
    /// `SV_LinkEdict (ent, false)` (`host_cmd.c:2101`).
    pub fn HostCmd_Glue_SVLinkEdict(ent: *mut c_void, touch_triggers: qboolean) -> c_int;
    /// `ED_Free (EDICT_NUM (i))` (`host_cmd.c:2112`).
    pub fn HostCmd_Glue_EDFree(ed: *mut c_void) -> c_int;
    /// `ED_RebuildFreeList (true)` (`host_cmd.c:2119`).
    pub fn HostCmd_Glue_EDRebuildFreeList(rebuild_all: qboolean) -> c_int;

    /// `Host_Reconnect_f ()` (`host_cmd.c:2149`) -- chunk B owns the body; the
    /// glue's C wrapper is what is guarded here.
    pub fn HostCmd_Glue_HostReconnect_f() -> c_int;
    // ==== end CHUNK C ====

    // ==== CHUNK D (host_cmd.c:2158-2649) ====

    /// `Quake/cl_main_glue.c:83-86` -- the three userinfo cvars
    /// `Host_Name_f`/`Host_Color_f` write through.
    pub static mut cl_name: cvar_t;
    pub static mut cl_topcolor: cvar_t;
    pub static mut cl_bottomcolor: cvar_t;

    /// `Quake/cmd.c` -- no error path (reads `cmd_argv`/`cmd_args`, no
    /// `Host_Error`).
    pub fn Cmd_Args() -> *const c_char;

    /// `host_cmd.c:2185, :2372-2373` -- `Cvar_Set (name, value)`. `Cvar_Set`
    /// can reach `Host_Error` through a callback (`Cvar_SetQuick` ->
    /// `Cvar_CallCallback`), matching the existing `SvMain_Glue_CvarSet`
    /// precedent (`sv_main_glue.c:305-329`) which this chunk's own C body
    /// mirrors rather than reuses (name-based seam, per the established
    /// per-chunk-duplication convention).
    pub fn HostCmd_Glue_CvarSet(name: *const c_char, value: *const c_char) -> c_int;

    /// `Cmd_ForwardToServer ()` -- `host_cmd.c:922`, `:984`, `:1029`, `:1075`,
    /// `:1131`, `:1188` (chunk B), `:2186`, `:2210`, `:2299`, `:2377`
    /// (chunk D), `:2665`, `:3234` (chunk E). Reaches
    /// `MSG_WriteByte`/`SZ_Print` -> `SZ_GetSpace`, which is `Host_Error`-
    /// capable on overflow (`cmd.c:954-984`).
    pub fn HostCmd_Glue_CmdForwardToServer() -> c_int;

    /// `host_cmd.c:2438, :2440` -- `PR_GetString (sv_player->v.netname)`.
    /// `PR_GetString` calls `Host_Error` on an invalid/non-existent string
    /// index (`pr_edict_arena.c:307-325`).
    pub fn HostCmd_Glue_PRGetString(num: c_int, out: *mut *const c_char) -> c_int;

    /// `host_cmd.c:2521-2527` -- the extended (17-64) spawn-parm write loop's
    /// `qcvm->globals[g->ofs] = host_client->spawn_parms[i]`, after
    /// `ED_FindGlobal` has already located `g` (write-direction mirror of the
    /// existing read-direction `SvMain_Glue_SpawnParmGlobal`,
    /// `sv_main_glue.c:429-435`). `ED_FindGlobal` and a bounded global-array
    /// store have no error path, so this is unguarded/void, matching its
    /// read-direction counterpart.
    pub fn HostCmd_Glue_SetSpawnParmGlobal(index: c_int, value: c_float);
    // ==== end CHUNK D ====

    // ==== CHUNK E (host_cmd.c:2650-3298) ====

    /// `Quake/common.h:480` -- `extern qboolean rogue, hipnotic;` alongside
    /// `standard_quake` (`cl_parse.rs:229-231` already declares that one).
    /// Mission-pack gates read by `Host_Give_f`'s "all" branch and its
    /// trailing `switch (sv_player->v.weapon)`.
    pub static mut hipnotic: qboolean;
    pub static mut rogue: qboolean;

    /// `Quake/cvar.h:143` -- `cvar_t *Cvar_FindVar (const char *var_name);`.
    /// A hash-table lookup with no error path; `Host_Setinfo_f`
    /// (`host_cmd.c:3226`) uses it to route a `src_command` "setinfo" through
    /// `Cvar_Set`/`Info_SetKey` depending on whether the key names a
    /// `CVAR_USERINFO` cvar.
    pub fn Cvar_FindVar(var_name: *const c_char) -> *mut cvar_t;

    /// `Quake/client.h:404` -- `void CL_StopPlayback (void);`. Confirmed
    /// non-raising: `cl_demo_glue.c`'s equivalent call notes it can only
    /// `fclose`, print through `Con_Printf`, and reach `Harness_DemoEnded`
    /// (which exits rather than longjmping). `Host_Stopdemo_f`
    /// (`host_cmd.c:3143`).
    pub fn CL_StopPlayback();

    /// `Quake/common.h:201-202` -- `void Info_Print (const char *info);` and
    /// `void Info_Enumerate (const char *info, void (*cb) (void *, const char
    /// *, const char *), void *cbctx);`. Both only parse the local info string
    /// and call `Con_Printf` / the callback -- no raise path (verified against
    /// `common.c`'s bodies). Used throughout `Host_Serverinfo_f`,
    /// `Host_Setinfo_f`, `Host_User_f`.
    pub fn Info_Print(info: *const c_char);
    pub fn Info_Enumerate(
        info: *const c_char,
        cb: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char),
        cbctx: *mut c_void,
    );

    /// `host_cmd.c:2971` (`Host_Viewmodel_f`): `SV_Precache_Model (m->name)`,
    /// capturing the returned precache index through `out`. Not the same as
    /// `SvMain_Glue_PrecacheModel` (`sv_main.rs`), whose only caller
    /// (`sv_main.c:1048`) discards that index.
    pub fn HostCmd_Glue_PrecacheModel(name: *const c_char, out: *mut c_int) -> c_int;

    /// `host_cmd.c:3007` (`PrintFrameName`): `Mod_Extradata (m)`. Guarded
    /// because `Mod_Extradata` -> `Mod_Extradata_CheckSkin` ->
    /// `Mod_LoadModel (mod, true)` reloads from disk when the cache slot was
    /// dropped and can `Host_Error`. `mod` and `out` are both opaque
    /// `qmodel_t *` / `aliashdr_t *` here; `quake-capi` casts through its
    /// `QModel` / `AliasHdr` mirrors.
    pub fn HostCmd_Glue_ModExtradata(model: *mut c_void, out: *mut *mut c_void) -> c_int;

    /// `host_cmd.c:3109`, `:3126` (`Host_Startdemos_f`, `Host_Demos_f`):
    /// `CL_NextDemo ()`.
    pub fn HostCmd_Glue_CLNextDemo() -> c_int;

    /// `host_cmd.c:3189`, `:3222` (`Host_Serverinfo_f`, `Host_Setinfo_f`):
    /// `SV_UpdateInfo (edict, keyname, value)`.
    pub fn HostCmd_Glue_SVUpdateInfo(
        edict: c_int,
        keyname: *const c_char,
        value: *const c_char,
    ) -> c_int;

    /// `host_cmd.c:3163` (`Info_ClientPrint_Callback`): `SV_ClientPrintf
    /// ("%20s: %s\n", key, val)`. `Info_ClientPrint_Callback` is passed to
    /// `Info_Enumerate` as a plain `void (*) (void *, const char *, const char
    /// *)`, so it cannot itself return a `Raise`; `quake-capi` writes this
    /// seam's result through `Info_Enumerate`'s own `cbctx` pointer, into a
    /// `Raise` that `Host_Serverinfo_f` / `Host_Setinfo_f` own on their stack
    /// and drain right after their `Info_Enumerate` call returns.
    pub fn HostCmd_Glue_SVClientPrintfKV(key: *const c_char, val: *const c_char) -> c_int;

    /// `host_cmd.c:3203` (`Host_Setinfo_f`): `SV_ClientPrintf ("Your
    /// Serverside User Info:\n")` -- a fixed literal, kept as its own
    /// hardcoded seam so the format string matches the original call site
    /// byte-for-byte rather than being routed through a generic wrapper.
    pub fn HostCmd_Glue_SVClientPrintfUserInfoHeader() -> c_int;
    // ==== end CHUNK E ====
}

/// `hostcmd_write_t` in `Quake/host_cmd_glue.c` -- one buffered `MSG_Write*`
/// call. `kind` selects the writer; the payload field a given kind reads is
/// documented next to `HostCmd_InvokeWriteBatch`.
///
/// 0 `MSG_WriteByte(i)`, 1 `MSG_WriteShort(i)`, 2 `MSG_WriteLong(i)`,
/// 3 `MSG_WriteFloat(f)`, 4 `MSG_WriteString(p)`, 5 `MSG_WriteAngle(f, u)`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostCmdWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub f: c_float,
    pub u: c_uint,
    pub p: *const c_void,
}
