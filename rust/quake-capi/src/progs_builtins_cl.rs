//! Client-coupled QuakeC builtins (Rust migration Phase 7 M5, wave 2 Group F):
//! `PF_cl_sound`, `PF_cl_ambientsound`, `PF_cl_precache_sound`,
//! `PF_cl_makestatic`, `PF_cl_particle`.
//!
//! These bodies come from `Quake/pr_cmds.c`'s `pr_csqcbuiltins[]` table, which
//! stays compiled as the oracle; the flip is Pattern C (one `builtin_t` table
//! slot at a time, through `pr_cmds_glue.c`'s `RUST_PF` wrappers gated by the
//! wave-1 `PF_RSH` macro).
//!
//! # Why this module is `host`-gated, not `progs`-gated
//!
//! `PF_cl_makestatic` reaches `cl.static_entities` / `entity_t`, which are
//! client-state, matching wave 1's rationale for why `-Duse_rust_progs` and
//! `-Duse_rust_host` are independent gates here. The C table rows use
//! `PF_RSH` and the C frame lives in `Quake/pr_cmds_cl_glue.c`, compiled with
//! the `use_rust_host` sources, so the module gate and the glue's compilation
//! condition are identical in every configuration -- the same reasoning wave
//! 1 recorded for Groups A/B/C even though this group never touches
//! `world.c` / `sv_move.c` / `sv_phys.c` directly.
//!
//! # ADR-009 audit
//!
//! Every raise reachable from this module is caught by a `Host_Guard` in
//! `pr_cmds_cl_glue.c` and reported as `PRBI_ERR_GUARD` with the guard status
//! as `detail`; `pr_cmds_glue.c`'s `PRBI_Raise` re-issues it from the C frame.
//! The raising seams are:
//!
//! * `PR_GetString` on every `G_STRING` handle (`PRBI_ClGlue_GetString`),
//! * `PR_CheckEmptyString`'s `PR_RunError ("Bad string")` in
//!   `PF_cl_precache_sound` (`PRBI_ClGlue_CheckEmptyString`),
//! * the whole `PF_cl_makestatic` body's `Mem_Realloc` / `Mem_Alloc` failure
//!   (`PRBI_ClGlue_MakeStatic`).
//!
//! `S_PrecacheSound`, `S_StartSound`, `S_StaticSound`,
//! `PScript_RunParticleEffect`, `PScript_RunParticleEffectTypeString` and
//! `R_RunParticleEffect` are called directly: none of them reach
//! `Host_Error` (only `S_FindName`'s pathological-path `Sys_Error`, which is
//! fatal and not a `Host_Guard`-caught longjmp).
//!
//! # ADR-006 / ADR-007
//!
//! No QuakeC is dispatched from any of these five builtins, so no Rust
//! reference needs to survive a call-out. `PF_cl_makestatic`'s entire body is
//! kept in the C glue helper rather than partially ported, because `entity_t`
//! and `cl` (`client_state_t`) have no ADR-011 mirror in Phase 7 -- porting
//! only the raising half and leaving the rest in C would still require an
//! `edict_t` / `entity_t` dual view on the Rust side for no benefit.
//!
//! # ADR-010
//!
//! `PF_cl_sound`'s origin fix-up (`VectorAdd` then `VectorMA`) and
//! `PF_cl_ambientsound`'s `vol = G_FLOAT (...) * 255` truncation are marked
//! `// COMPAT:` at their sites below; no `f32::` transcendental methods are
//! used (none of these five builtins call one in C).

use core::ffi::{c_char, c_float, c_int, c_void};
use core::ptr;

use quake_c_sys::progs_builtins_cl as g;
use quake_c_sys::world as wg;
use quake_types::progs::{
    Edict, QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_PARM3, OFS_PARM4, OFS_RETURN,
};

use crate::progs_builtins_sv::{guarded, run_sv, SvConsole, SvRaise, SvResult};

/* ---------------------------------------------------------------------------
 * progs.h macro equivalents, duplicated locally: `progs_builtins_sv.rs`'s
 * copies are private to that module, and wave 2's addendum only makes its
 * `pub(crate)` plumbing (PRBI_OK / PRBI_ERR_GUARD / SvRaise / SvResult /
 * guarded / SvConsole / run_sv) shared, not these.
 */

#[inline]
unsafe fn globals(vm: *mut QcVm) -> *mut c_float {
    // SAFETY: the ambient qcvm is live for the duration of a builtin (ADR-008).
    unsafe { (*vm).globals }
}

/// `progs.h` `G_FLOAT (o)`
#[inline]
unsafe fn g_float(vm: *mut QcVm, ofs: usize) -> c_float {
    // SAFETY: `ofs` is a fixed OFS_* slot inside the globals block.
    unsafe { *globals(vm).add(ofs) }
}

/// `progs.h` `G_INT (o)`
#[inline]
unsafe fn g_int(vm: *mut QcVm, ofs: usize) -> c_int {
    // SAFETY: as `g_float`; the C macro type-puns the same word.
    unsafe { *globals(vm).add(ofs).cast::<c_int>() }
}

/// `progs.h` `G_VECTOR (o)`
#[inline]
unsafe fn g_vector(vm: *mut QcVm, ofs: usize) -> *mut c_float {
    // SAFETY: as `g_float`.
    unsafe { globals(vm).add(ofs) }
}

/// `progs.h` `PROG_TO_EDICT (e)` -- byte offset, no bounds check, exactly like
/// the C macro.
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
// are UB in C and saturate in Rust; the same shim `progs_builtins_sv.rs` uses.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/// `NUM_FOR_EDICT` through `world_glue.c`'s guard -- it Host_Errors on a bad
/// pointer (`pr_edict.c`), so it cannot be expanded in a Rust frame (ADR-009).
#[inline]
unsafe fn num_for_edict(e: *mut Edict) -> Result<c_int, SvRaise> {
    let mut out: c_int = 0;
    // SAFETY: the glue writes `out` only on success.
    let status = unsafe { wg::World_Glue_NumForEdict(e.cast::<c_void>(), &mut out) };
    guarded(status)?;
    Ok(out)
}

/// `G_STRING (o)` through `PRBI_ClGlue_GetString` -- `PR_GetString` Host_Errors
/// on an out-of-range negative handle (ADR-009).
#[inline]
unsafe fn get_string(handle: c_int) -> Result<*const c_char, SvRaise> {
    let mut out: *const c_char = ptr::null();
    // SAFETY: the glue writes `out` only on success.
    let status = unsafe { g::PRBI_ClGlue_GetString(handle, &mut out) };
    guarded(status)?;
    Ok(out)
}

/// `PR_CheckEmptyString (s)` through `PRBI_ClGlue_CheckEmptyString` --
/// `PR_RunError ("Bad string")` on an empty/whitespace-leading string
/// (ADR-009).
#[inline]
unsafe fn check_empty_string(s: *const c_char) -> SvResult {
    // SAFETY: `s` is the pointer `get_string` just returned; still NUL
    // terminated and live for the duration of this builtin (ADR-008).
    guarded(unsafe { g::PRBI_ClGlue_CheckEmptyString(s) })
}

/* ---------------------------------------------------------------------------
 * Group F -- client set (pr_cmds.c:1779, :1806, :1872, :1884, :1931).
 */

/// `pr_cmds.c:1779` `PF_cl_sound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_sound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let entity = g_edict(vm, OFS_PARM0);
            // COMPAT: ADR-010 -- `channel = G_FLOAT (OFS_PARM1);` (pr_cmds.c),
            // an implicit C float->int conversion into the local `int channel`.
            let channel = as_int(g_float(vm, OFS_PARM1));
            let sample = get_string(g_int(vm, OFS_PARM2))?;
            let volume = g_float(vm, OFS_PARM3);
            let attenuation = g_float(vm, OFS_PARM4);

            // `NUM_FOR_EDICT (entity); entnum *= -1;` -- fullcsqc fixme
            // preserved verbatim (pr_cmds.c:1789-1791): the entity's own
            // `entnum` field is never consulted here.
            let mut entnum = num_for_edict(entity)?;
            entnum *= -1;

            // COMPAT: ADR-010 -- `VectorAdd (mins, maxs, origin);
            // VectorMA (origin_v, 0.5, origin, origin);` preserved in the
            // exact C operation order (no reassociation).
            let mins = (*entity).v.mins;
            let maxs = (*entity).v.maxs;
            let ent_origin = (*entity).v.origin;
            let mut origin = [mins[0] + maxs[0], mins[1] + maxs[1], mins[2] + maxs[2]];
            origin = [
                ent_origin[0] + 0.5 * origin[0],
                ent_origin[1] + 0.5 * origin[1],
                ent_origin[2] + 0.5 * origin[2],
            ];

            let sfx = g::S_PrecacheSound(sample);
            g::S_StartSound(
                entnum,
                channel,
                sfx,
                origin.as_mut_ptr(),
                volume,
                attenuation,
            );
            Ok(())
        })
    }
}

/// `pr_cmds.c:1806` `PF_cl_ambientsound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_ambientsound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let pos = g_vector(vm, OFS_PARM0);
            let samp = get_string(g_int(vm, OFS_PARM1))?;
            // COMPAT: ADR-010 -- `vol = G_FLOAT (OFS_PARM2) * 255;` (pr_cmds.c)
            // computed in float exactly as C does; `S_StaticSound`'s `vol`
            // parameter is `int` (snd_dma.c:619), so the implicit C
            // float->int conversion at the call site is made explicit here.
            let vol = g_float(vm, OFS_PARM2) * 255.0;
            let attenuation = g_float(vm, OFS_PARM3);

            let sfx = g::S_PrecacheSound(samp);
            g::S_StaticSound(sfx, pos, as_int(vol), attenuation);
            Ok(())
        })
    }
}

/// `pr_cmds.c:1872` `PF_cl_precache_sound`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_precache_sound(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let handle = g_int(vm, OFS_PARM0);
            let s = get_string(handle)?;

            // `G_INT (OFS_RETURN) = G_INT (OFS_PARM0);` -- echoes the handle
            // back before validating it, exactly as pr_cmds.c:1875 does.
            *globals(vm).add(OFS_RETURN).cast::<c_int>() = handle;

            check_empty_string(s)?;

            // precache sounds are optional in quake's sound system. NULL is a
            // valid response so don't check (pr_cmds.c:1880-1881).
            g::S_PrecacheSound(s);
            Ok(())
        })
    }
}

/// `pr_cmds.c:1884` `PF_cl_makestatic`.
///
/// The entire body is delegated to `PRBI_ClGlue_MakeStatic` (ADR-007):
/// `entity_t` / `cl.static_entities` have no ADR-011 mirror in Phase 7.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_makestatic(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let ent = g_edict(vm, OFS_PARM0);
            guarded(g::PRBI_ClGlue_MakeStatic(ent.cast::<c_void>()))
        })
    }
}

/// `pr_cmds.c:1931` `PF_cl_particle`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_particle(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let org = g_vector(vm, OFS_PARM0);
            let dir = g_vector(vm, OFS_PARM1);
            let color = g_float(vm, OFS_PARM2);
            let mut count = g_float(vm, OFS_PARM3);

            if count == 255.0 {
                let ran =
                    g::PScript_RunParticleEffectTypeString(org, dir, 1.0, c"te_explosion".as_ptr());
                count = if ran == 0 { 0.0 } else { 1024.0 };
            } else {
                // COMPAT: ADR-010 -- `PScript_RunParticleEffect (org, dir,
                // color, count)` (pr_cmds.c) implicitly converts the float
                // locals `color` / `count` to the callee's `int` parameters
                // (glquake.h).
                let ran = g::PScript_RunParticleEffect(org, dir, as_int(color), as_int(count));
                count = if ran == 0 { 0.0 } else { count };
            }
            // COMPAT: ADR-010 -- `R_RunParticleEffect (org, dir, color,
            // count)` (pr_cmds.c) implicitly converts the same two floats to
            // the callee's `int` parameters (render.h).
            g::R_RunParticleEffect(org, dir, as_int(color), as_int(count));
            Ok(())
        })
    }
}
