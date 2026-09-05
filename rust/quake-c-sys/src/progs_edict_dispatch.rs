//! `Quake/pr_edict_dispatch_glue.c` declarations (Rust migration Phase 7 M5
//! T5.2) plus two Phase-6-era engine seams (`COM_ParseEx`,
//! `SV_Precache_Sound`) and one leaf builtin (`PF_SV_ForceParticlePrecache`)
//! that `ED_ParseGlobals`/`ED_ParseEdict` need but no earlier phase bound.
//!
//! ADR-011: engine C symbols are declared only in this crate.
//!
//! `SV_Precache_Sound`/`PF_SV_ForceParticlePrecache` are safe to call
//! directly (no Host_Guard) *only* on the call path
//! `quake-capi::progs_edict_dispatch` actually uses them from: both are
//! reached exclusively when `sv.state == ss_loading` (mirroring
//! `Quake/pr_edict.c:860-887`'s own `sv.state == ss_loading` guards on these
//! same three call sites), and both functions gate their only
//! `Host_Error`-reachable statements (`MSG_Write*`/`SV_Multicast` into
//! `sv.reliable_datagram`/`sv.multicast`) behind `if (sv.state !=
//! ss_loading)` -- dead code here. `SV_Precache_Model` is different: its
//! `Mod_ForName (s, i == 1)` call is *not* gated on `sv.state`, and
//! `Mod_LoadModel` (`Quake/gl_model.c:531`) `Host_Error`s when `crash` (i.e.
//! `i == 1`) is true and the file is missing, so it is wrapped behind
//! `PREdictDispatch_Glue_PrecacheModel`'s `Host_Guard` below instead of
//! declared as a plain extern (ADR-009 rule 3; see the T5.2 manifest for the
//! full evidence trail).
//!
//! This file is a stand-in for the main session: the T5.2 manifest asks it to
//! fold these declarations into `bindings_wrapper.h` / `gen_c_bindings.sh`'s
//! allowlist (so cbindgen produces them into `generated.rs`) and then delete
//! this file.

use core::ffi::{c_char, c_int, c_ushort};

extern "C" {
    /// C: `const char *COM_ParseEx (const char *data, cpe_mode mode)`
    /// (`Quake/common.h:276`). `mode` is `cpe_mode` (`Quake/common.h:269-273`):
    /// `0 = CPE_NOTRUNC`, `1 = CPE_ALLOWTRUNC`.
    pub fn COM_ParseEx(data: *const c_char, mode: c_int) -> *const c_char;

    /// C: `int SV_Precache_Sound (const char *s)` (`Quake/progs.h:106`). Safe
    /// to call directly only when `sv.state == ss_loading` -- see the module
    /// doc comment.
    pub fn SV_Precache_Sound(s: *const c_char) -> c_int;
    /// C: `int PF_SV_ForceParticlePrecache (const char *s)`
    /// (`Quake/progs.h:104`). As `SV_Precache_Sound`, safe only when
    /// `sv.state == ss_loading`.
    pub fn PF_SV_ForceParticlePrecache(s: *const c_char) -> c_int;

    /// `Quake/pr_edict_dispatch_glue.c`: `ED_FindGlobal` hash lookup. Returns
    /// `false` when absent, leaving the three out-params untouched.
    pub fn PREdictDispatch_Glue_FindGlobal(
        name: *const c_char,
        out_type: *mut c_ushort,
        out_ofs: *mut c_ushort,
        out_s_name: *mut c_int,
    ) -> crate::qboolean;
    /// As above, over `ED_FindField`.
    pub fn PREdictDispatch_Glue_FindField(
        name: *const c_char,
        out_type: *mut c_ushort,
        out_ofs: *mut c_ushort,
        out_s_name: *mut c_int,
    ) -> crate::qboolean;
    /// `Quake/pr_edict_dispatch_glue.c`: `sv.state == ss_loading`.
    pub fn PREdictDispatch_Glue_ServerLoading() -> crate::qboolean;

    /// `Host_Guard`-wrapped `SV_Precache_Model (s)` (`Quake/progs.h:105`):
    /// `*out` receives the precache index on `HOST_GUARD_OK`, untouched
    /// otherwise. See the module doc comment for why this one needs guarding
    /// and its siblings do not.
    pub fn PREdictDispatch_Glue_PrecacheModel(s: *const c_char, out: *mut c_int) -> c_int;
}
