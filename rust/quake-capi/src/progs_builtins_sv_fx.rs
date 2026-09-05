//! Group E world-effect QuakeC builtins (Rust migration Phase 7 M5 wave 2):
//! `PF_sound`, `PF_particle`, `PF_sv_ambientsound`, `PF_sv_lightstyle`,
//! `PF_sv_makestatic`, `PF_sv_setspawnparms`, `PF_sv_changelevel`,
//! `PF_sv_precache_sound`, `PF_sv_precache_model`, `PF_sv_finalefinished`,
//! `PF_sv_localsound`, `PF_sv_CheckPlayerEXFlags`.
//!
//! These bodies come from `Quake/pr_cmds.c`, which stays compiled as the
//! oracle; the flip is Pattern C (one `builtin_t` table slot at a time
//! through `pr_cmds_glue.c`'s `RUST_PF` wrappers, gated `PF_RSH` per the M5
//! wave 2 addendum).
//!
//! # Why this module is `host`-gated
//!
//! None of these builtins reach `world.c` / `sv_move.c` / `sv_phys.c`
//! directly, but `sv` / `svs` (`server_t` / `server_static_t`) have no
//! ADR-011 mirror in Phase 7, so every non-trivial body below is kept whole
//! in C -- matching wave 1's `PRBI_SvGlue_SetModelLookup` precedent for
//! `sv.model_precache`. `-Duse_rust_progs` and `-Duse_rust_host` are
//! independent Meson options, so this module is gated on the `host` feature
//! like `progs_builtins_sv`, and the C table rows use `PF_RSH`.
//!
//! # ADR-009 audit
//!
//! Every raise reachable from this module is caught by a `Host_Guard` in
//! `pr_cmds_sv_fx_glue.c` and reported as `PRBI_ERR_GUARD` with the guard
//! status as `detail`; `pr_cmds_glue.c`'s `PRBI_Raise` re-issues it from the
//! C frame. The raising seams, all guarded conservatively because `G_STRING`
//! (`PR_GetString`, `pr_edict_arena.c:307`) can `Host_Error` on a cleared
//! known-string even though the ordinary QC-constant case never does:
//!
//! * `PF_sound` -> the `G_STRING` fetch, `SV_StartSound` (bad
//!   volume/attenuation/channel, or a bad `entity` via its internal
//!   `NUM_FOR_EDICT`);
//! * `PF_sv_ambientsound` -> the `G_STRING` fetch, the `sv.ambientsounds`
//!   growth's `PR_RunError` on a failed `Mem_Realloc`;
//! * `PF_sv_lightstyle` -> the `G_STRING` fetch only (never raises
//!   otherwise);
//! * `PF_sv_makestatic` -> the `sv.static_entities` growth's `PR_RunError`,
//!   `ED_Free` (can itself `Host_Error`);
//! * `PF_sv_setspawnparms` -> `NUM_FOR_EDICT`, `"Entity is not a client"`;
//! * `PF_sv_changelevel` -> the `G_STRING` fetch (the
//!   `svs.changelevel_issued` check-and-set itself cannot raise; done here
//!   via the existing `PRBI_Glue_ChangelevelIssued`);
//! * `PF_sv_precache_sound` / `PF_sv_precache_model` -> the `G_STRING`
//!   fetch, `PR_CheckEmptyString`'s `"Bad string"`, the overflow
//!   `PR_RunError`s;
//! * `PF_sv_localsound` -> `NUM_FOR_EDICT`, the `G_STRING` fetch.
//!
//! `PF_particle` never raises (`SV_StartParticle`, `sv_main.c:1231`, clamps
//! its inputs and writes network bytes directly) and uses no `G_STRING` /
//! `G_EDICTNUM`, so it calls the real function directly with no guard.
//! `PF_sv_finalefinished` / `PF_sv_CheckPlayerEXFlags` are
//! `G_FLOAT (OFS_RETURN) = 0` and reach nothing else.
//!
//! # Console output
//!
//! None of these builtins print from a bare Rust frame: every print
//! (`Con_Printf`, `Con_Warning`, `Con_DWarning`, `PR_RunWarning`) happens
//! inside one of the `PRBI_FxGlue_*` `Host_Guard` C frames above, exactly
//! like wave 1's `PRBI_SvInvokeWarnNanTrace`. `progs_builtins_sv`'s deferred
//! `SvConsole` is therefore unused by this module.
//!
//! # ADR-010
//!
//! `PF_sound`'s `volume = G_FLOAT (OFS_PARM3) * 255;` and `PF_sv_lightstyle`'s
//! `style = G_FLOAT (OFS_PARM0);` are C's implicit float->int truncation of a
//! float *after* any arithmetic; both are marked `// COMPAT:` at the site
//! below, matching the `as_int` shim `progs_builtins_sv.rs` / `sv_phys.rs`
//! already use. No float reassociation is performed anywhere in this module.

use core::ffi::{c_float, c_int, c_void};

use quake_c_sys::progs_builtins_sv_fx as g;
use quake_types::progs::{
    Edict, QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_PARM3, OFS_PARM4, OFS_RETURN,
};

use crate::progs_builtins_sv::{guarded, run_sv};

/* ---------------------------------------------------------------------------
 * progs.h macro equivalents, duplicated locally (module-private in
 * progs_builtins_sv.rs, so not reusable across modules -- same convention
 * `sv_move.rs` / `sv_phys.rs` already follow for their own copies).
 */

#[inline]
unsafe fn globals(vm: *mut QcVm) -> *mut c_float {
    // SAFETY: `vm` is the live ambient qcvm (ADR-008).
    unsafe { (*vm).globals }
}

/// `progs.h` `G_FLOAT (o)`
#[inline]
unsafe fn g_float(vm: *mut QcVm, ofs: usize) -> c_float {
    // SAFETY: callers pass an in-range `OFS_PARM*`/`OFS_RETURN`.
    unsafe { *globals(vm).add(ofs) }
}

/// `progs.h` `G_INT (o)`
#[inline]
unsafe fn g_int(vm: *mut QcVm, ofs: usize) -> c_int {
    // SAFETY: as `g_float`.
    unsafe { *globals(vm).add(ofs).cast::<c_int>() }
}

/// `progs.h` `G_VECTOR (o)`
#[inline]
unsafe fn g_vector(vm: *mut QcVm, ofs: usize) -> *mut c_float {
    // SAFETY: as `g_float`; the 3 floats at `ofs..ofs+3` stay in bounds for
    // an `OFS_PARM*` offset.
    unsafe { globals(vm).add(ofs) }
}

/// `progs.h` `PROG_TO_EDICT (e)` -- byte offset, no bounds check, exactly
/// like the C macro.
#[inline]
unsafe fn prog_to_edict(vm: *mut QcVm, p: c_int) -> *mut Edict {
    // SAFETY: pointer arithmetic only.
    unsafe {
        (*vm)
            .edicts
            .cast::<u8>()
            .wrapping_offset(p as isize)
            .cast::<Edict>()
    }
}

/// `progs.h` `G_EDICT (o)`
#[inline]
unsafe fn g_edict(vm: *mut QcVm, ofs: usize) -> *mut Edict {
    // SAFETY: as `g_int`.
    unsafe { prog_to_edict(vm, g_int(vm, ofs)) }
}

// COMPAT: ADR-010 -- C's implicit float->int conversion. Out-of-range values
// are UB in C and saturate in Rust; the same shim progs_builtins_sv.rs and
// sv_phys.rs use.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/* ---------------------------------------------------------------------------
 * Group E -- world-effect builtins.
 */

/// `pr_cmds.c:614` `PF_particle`. Never raises: `SV_StartParticle` clamps
/// its inputs and writes network bytes directly (`sv_main.c:1231-1256`).
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_particle(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            let dir = g_vector(vm, OFS_PARM1);
            let color = as_int(g_float(vm, OFS_PARM2));
            let count = as_int(g_float(vm, OFS_PARM3));
            g::SV_StartParticle(org, dir, color, count);
            Ok(())
        })
    }
}

/// `pr_cmds.c:692` `PF_sound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let entity = g_edict(vm, OFS_PARM0).cast::<c_void>();
            // COMPAT: ADR-010 -- `channel = G_FLOAT (OFS_PARM1);` truncates.
            let channel = as_int(g_float(vm, OFS_PARM1));
            let sample_handle = g_int(vm, OFS_PARM2);
            // COMPAT: ADR-010 -- `volume = G_FLOAT (OFS_PARM3) * 255;`
            // multiplies in float, then truncates to `int` on assignment.
            let volume = as_int(g_float(vm, OFS_PARM3) * 255.0);
            let attenuation = g_float(vm, OFS_PARM4);
            guarded(g::PRBI_FxGlue_Sound(
                entity,
                channel,
                sample_handle,
                volume,
                attenuation,
            ))
        })
    }
}

/// `pr_cmds.c:633` `PF_sv_ambientsound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_ambientsound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            let sample_handle = g_int(vm, OFS_PARM1);
            let vol = g_float(vm, OFS_PARM2);
            let attenuation = g_float(vm, OFS_PARM3);
            guarded(g::PRBI_FxGlue_AmbientSound(
                pos,
                sample_handle,
                vol,
                attenuation,
            ))
        })
    }
}

/// `pr_cmds.c:1364` `PF_sv_lightstyle`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_lightstyle(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            // COMPAT: ADR-010 -- `style = G_FLOAT (OFS_PARM0);` truncates.
            let style = as_int(g_float(vm, OFS_PARM0));
            let val_handle = g_int(vm, OFS_PARM1);
            guarded(g::PRBI_FxGlue_LightStyle(style, val_handle))
        })
    }
}

/// `pr_cmds.c:1708` `PF_sv_makestatic`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_makestatic(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let ent = g_edict(vm, OFS_PARM0).cast::<c_void>();
            guarded(g::PRBI_FxGlue_MakeStatic(ent))
        })
    }
}

/// `pr_cmds.c:1743` `PF_sv_setspawnparms`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_setspawnparms(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let ent = g_edict(vm, OFS_PARM0).cast::<c_void>();
            guarded(g::PRBI_FxGlue_SetSpawnParms(ent))
        })
    }
}

/// `pr_cmds.c:1766` `PF_sv_changelevel`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_changelevel(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            // `svs.changelevel_issued` check-and-set (pr_cmds.c:1771-1773):
            // `PRBI_Glue_ChangelevelIssued` returns the old value and only
            // ever sets the flag *true*, so passing `true` here reproduces
            // "if already issued, return; else set it and continue" exactly.
            if quake_c_sys::PRBI_Glue_ChangelevelIssued(true) {
                return Ok(());
            }
            let level_handle = g_int(vm, OFS_PARM0);
            guarded(g::PRBI_FxGlue_ChangeLevel(level_handle))
        })
    }
}

/// `pr_cmds.c:1188` `PF_sv_precache_sound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_precache_sound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let handle = g_int(vm, OFS_PARM0);
            // `G_INT (OFS_RETURN) = G_INT (OFS_PARM0)` (pr_cmds.c:1193) is
            // written BEFORE `PR_CheckEmptyString`/`SV_Precache_Sound` run,
            // i.e. unconditionally, even on the raise path below -- match
            // that exact statement order rather than writing it only after
            // the guarded call succeeds (COMPAT fix: an earlier draft wrote
            // this after the `?`, which left OFS_RETURN unwritten on a
            // caught raise where the real engine leaves the pass-through
            // handle in place).
            *globals(vm).add(OFS_RETURN).cast::<c_int>() = handle;
            guarded(g::PRBI_FxGlue_PrecacheSound(handle))?;
            Ok(())
        })
    }
}

/// `pr_cmds.c:1225` `PF_sv_precache_model`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_precache_model(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let handle = g_int(vm, OFS_PARM0);
            // Same COMPAT fix and ordering rationale as
            // `quake_rs_pf_sv_precache_sound` above: pr_cmds.c:1231 writes
            // `G_INT (OFS_RETURN) = G_INT (OFS_PARM0)` before the empty
            // string check / precache scan, unconditionally.
            *globals(vm).add(OFS_RETURN).cast::<c_int>() = handle;
            guarded(g::PRBI_FxGlue_PrecacheModel(handle))?;
            Ok(())
        })
    }
}

/// `pr_cmds.c:1845` `PF_sv_finalefinished` (2021 re-release). Reaches
/// nothing but the return slot.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_finalefinished(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            *globals(vm).add(OFS_RETURN) = 0.0;
            Ok(())
        })
    }
}

/// `pr_cmds.c:1849` `PF_sv_CheckPlayerEXFlags` (2021 re-release). Reaches
/// nothing but the return slot.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
///
/// COMPAT/wiring: exported name preserves `PF_sv_CheckPlayerEXFlags`'s exact
/// case (matching the established `quake_rs_pf_sv_WriteByte` precedent in
/// `progs_builtins.rs`), because `pr_cmds_glue.c`'s `RUST_PF (name)` macro
/// token-pastes `quake_rs_pf_##name` -- a snake_cased export here would leave
/// `RUST_PF (sv_CheckPlayerEXFlags)` unable to find this symbol at T5.3. An
/// earlier draft exported `quake_rs_pf_sv_check_player_ex_flags`, a real bug
/// found while re-verifying against `pr_cmds_glue.c:340-347`.
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn quake_rs_pf_sv_CheckPlayerEXFlags(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            *globals(vm).add(OFS_RETURN) = 0.0;
            Ok(())
        })
    }
}

/// `pr_cmds.c:1857` `PF_sv_localsound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_localsound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let ent = g_edict(vm, OFS_PARM0).cast::<c_void>();
            let sample_handle = g_int(vm, OFS_PARM1);
            guarded(g::PRBI_FxGlue_LocalSound(ent, sample_handle))
        })
    }
}
