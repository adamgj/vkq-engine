//! `Quake/pr_ext.c` declarations (Rust migration Phase 7 M9f).
//!
//! ADR-011: engine C symbols are declared only in this crate. Unlike the
//! other builtin groups, `pr_ext.c`'s seams cannot live in a `*_glue.c` of
//! their own: `SV_Multicast` is `static` inside `pr_ext.c` (`:72`, `:4216`),
//! so its `Host_Guard` trampoline has to be in that translation unit. Both
//! forwarders below are therefore defined in `pr_ext.c` itself, under
//! `#ifdef USE_RUST_HOST`.

use core::ffi::{c_float, c_int, c_uint};

use crate::cvar_t;

extern "C" {
    /* ---- guarded seams (ADR-009 rule 3) ---- */

    /// `pr_ext.c:4216` `SV_Multicast (to, org, msg_entity, requireext2)`,
    /// guarded: every arm ends in `SZ_Write`, which `Host_Error`s when the
    /// destination sizebuf overflows (`net_msg_glue.c:71`). `org` is read
    /// only, and only by the `MULTICAST_PVS_*` arms.
    pub fn PRExt_Glue_SVMulticast(
        to: c_int,
        org: *mut c_float,
        msg_entity: c_int,
        requireext2: c_uint,
    ) -> c_int;

    /// `pr_ext.c:4580` `COM_Effectinfo_Enumerate (PF_SV_ForceParticlePrecache)`,
    /// guarded: it reads `effectinfo.txt` and the callback writes to
    /// `sv.multicast`, both `Host_Error`-capable. The callback stays C --
    /// `pr_edict.c:978` and `quake-capi/src/progs_edict_dispatch.rs` call it
    /// by that exact name.
    pub fn PRExt_Glue_EffectinfoEnumerate() -> c_int;

    /* ---- plain engine globals ---- */

    /// `r_part.c` `r_particledesc`, read by `PF_sv_particleeffectnum`
    /// (`pr_ext.c:4572`) through its own function-scope `extern` declaration.
    pub static mut r_particledesc: cvar_t;
}
