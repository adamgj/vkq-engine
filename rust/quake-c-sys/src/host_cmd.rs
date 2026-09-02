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

use core::ffi::{c_int, c_uint, c_void};

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

    // =======================================================================
    // Guarded seams (ADR-009 rule 3). Appended per chunk during T8.3; keep the
    // chunk banners so the merge stays reviewable.
    // =======================================================================
}
