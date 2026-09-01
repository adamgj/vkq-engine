//! `Quake/chase_glue.c` declarations (Rust migration Phase 7 M7, T7.2a).
//!
//! ADR-011: engine C symbols are declared only in this crate. `Quake/chase.c`
//! defined four cvars; under `-Duse_rust_host` that storage moves to
//! `Quake/chase_glue.c` so `Cvar_RegisterVariable` keeps receiving stable
//! `cvar_t` addresses and `Quake/view.c`'s successor plus `gl_rmain.c`/
//! `menu.c` keep resolving the plain names.
//!
//! `SV_RecursiveHullCheck` is already a Rust export (`quake-capi`'s `world`
//! module, Phase 7 M3) and `AngleVectors`/`VectorAngles`/`VectorLength` are
//! `quake-math`, so `chase.c`'s only remaining C dependency is the registration
//! trampoline.

use core::ffi::c_int;

extern "C" {
    /* Quake/chase_glue.c data -- chase.c:25-28. */
    pub static mut chase_back: crate::cvar_t;
    pub static mut chase_up: crate::cvar_t;
    pub static mut chase_right: crate::cvar_t;
    pub static mut chase_active: crate::cvar_t;

    /// ADR-009: wraps one `Cvar_RegisterVariable`, which is itself a
    /// `Host_Reraise` wrapper under `-Duse_rust_cvar`. Returns a `Host_Guard`
    /// status (0 = returned normally, 1/2 = a jump was caught).
    pub fn Chase_Glue_RegisterVariable(var: *mut crate::cvar_t) -> c_int;
}
