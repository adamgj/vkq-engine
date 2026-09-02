//! `pr_ext.c` particle group (Phase 7 M9f group E): particleeffectnum,
//! trailparticles and pointparticles (`Quake/pr_ext.c:4539-4798`), plus the
//! two non-`static` helpers `PF_CL_ForceParticlePrecache` and
//! `PF_CL_GetParticle` that nothing outside `pr_ext.c` calls.
//!
//! `PF_SV_ForceParticlePrecache` deliberately stays C: `progs.h:104` declares
//! it, `pr_edict.c:978` and `progs_edict_dispatch.rs` reach it by that exact
//! name, and `COM_Effectinfo_Enumerate` takes it as a function pointer.
//!
//! # Why this module is `host`-gated, not `progs`-gated
//! Every builtin here touches `sv` or `cl` -- the precache tables, `sv.state`,
//! `sv.multicast` and `sv.protocolflags` -- so it can only be built where the
//! client/server strata are Rust-owned. That is `use_rust_host`, exactly as for
//! `progs_builtins_sv.rs` and `progs_builtins_cl.rs`.
//!
//! # ADR-009 audit
//! Four raise-capable callee classes, all reached through a C frame:
//! * `MSG_WriteByte/Short/String/Coord` into `sv.multicast` -- `SZ_GetSpace`
//!   `Host_Error`s on overflow (`net_msg_glue.c:71`). Batched through the
//!   existing guarded `SvSend_Glue_WriteBatch`.
//! * `SV_Multicast` -- `static` inside `pr_ext.c`, and every arm ends in
//!   `SZ_Write`. Reached through `PRExt_Glue_SVMulticast`, the `Host_Guard`
//!   trampoline that had to be added to `pr_ext.c` itself.
//! * `COM_Effectinfo_Enumerate (PF_SV_ForceParticlePrecache)` -- reads a file
//!   and the callback writes to `sv.multicast`. Reached through
//!   `PRExt_Glue_EffectinfoEnumerate`.
//! * `G_STRING` / `NUM_FOR_EDICT` / `PScript_*` -- the existing guarded
//!   `PRBI_MsgGlue_GetString`, `World_Glue_NumForEdict`,
//!   `ClMain_Glue_ParticleTrail`, `ClMain_Glue_RunParticleEffectState` and
//!   `ClMain_Glue_FindParticleType` seams.
//!
//! The two `PR_RunError` sites return
//! `PRBI_ERR_{SV,CL}_PARTICLEEFFECTNUM_OVERFLOW` instead; `PRBI_Raise` in
//! `pr_cmds_glue.c` -- a C frame -- issues the original message as a literal
//! (ADR-009 rule 2).
//!
//! # ADR-006 / ADR-007
//! No edict reference lives across a builtin dispatch: `PF_cl_trailparticles`
//! converts its `edict_t *` to a number through the guarded seam and keeps only
//! the number. `sv`/`cl` are the Rust-owned storage closed at M6/M7.
//!
//! # ADR-005
//! Clean. The only formatted output is
//! `Con_Warning ("PF_sv_particleeffectnum(%s): ...", s)`, a plain `%s`, which
//! is assembled here as bytes and handed to `Con_Warning` through
//! [`SvConsole`]'s own `"%s"`.
//!
//! # ADR-010 and the bounds-panic audit
//! No libm and no arithmetic beyond comparisons. `panic = "abort"` sites
//! audited:
//! * every `particle_precache` / `local_particle_precache` index comes from a
//!   `1..MAX_PARTICLETYPES` loop or from an explicit `idx >= MAX_PARTICLETYPES`
//!   test, so no bounds check can fire;
//! * `MAX_EDICTS * (unsigned int) qcvm->edict_size` is a C unsigned multiply
//!   that is allowed to wrap -- `wrapping_mul` here;
//! * `pr_ext_warned_particleeffectnum++` is a plain `int` post-increment C lets
//!   overflow -- `wrapping_add` here;
//! * `-NUM_FOR_EDICT (ent)` is `wrapping_neg`;
//! * `(int) G_FLOAT (...)` uses the shared `as_int` shim, which documents the
//!   C-UB / Rust-saturation difference.
//!
//! Console ordering: `SvConsole` flushes after the Rust frame returns, so the
//! warning lands after this builtin's network writes rather than before them.
//! That is the established `progs_builtins_sv.rs` deviation, not a new one --
//! `Con_Printf` is not a leaf (`console.c:1282` reaches `SCR_UpdateScreen`).

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_main as clg;
use quake_c_sys::pr_ext as extg;
use quake_c_sys::progs_builtins_sv_msg as msgg;
use quake_c_sys::sv_send as sendg;
use quake_types::host::{ClientState, ParticlePrecacheEntry, Server, MAX_PARTICLETYPES};
use quake_types::progs::{
    Edict, QcVm, MAX_EDICTS, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_PARM3, OFS_RETURN,
};

use crate::cl_main::cl;
use crate::progs_builtins_sv::{guarded, run_sv, SvConsole, SvRaise, SvResult};
use crate::sv_main::sv;

/* ---------------------------------------------------------------------------
 * Constants, duplicated locally the way progs_builtins_sv.rs does.
 */

/// `pr_cmds_glue.c` `PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW` --
/// `PR_RunError ("PF_sv_particleeffectnum: overflow")`.
const PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW: c_int = 9;
/// `pr_cmds_glue.c` `PRBI_ERR_CL_PARTICLEEFFECTNUM_OVERFLOW` --
/// `PR_RunError ("PF_cl_particleeffectnum: overflow")`.
const PRBI_ERR_CL_PARTICLEEFFECTNUM_OVERFLOW: c_int = 10;

/// `protocol.h:322` `svcdp_precache`.
const SVCDP_PRECACHE: c_int = 54;
/// `protocol.h:331` `svcdp_trailparticles`.
const SVCDP_TRAILPARTICLES: c_int = 60;
/// `protocol.h:332` `svcdp_pointparticles`.
const SVCDP_POINTPARTICLES: c_int = 61;
/// `protocol.h:333` `svcdp_pointparticles1`.
const SVCDP_POINTPARTICLES1: c_int = 62;
/// `protocol.h:60` `PEXT2_REPLACEMENTDELTAS`.
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;

/// `pr_ext.c:59-70` `multicast_t`.
const MULTICAST_PHS_U: c_int = 1;
const MULTICAST_PVS_U: c_int = 2;
const MULTICAST_ALL_R: c_int = 3;

/// `server.h:44` `ss_loading` (first enumerator).
const SS_LOADING: c_int = 0;

/// `glquake.h:107` `P_INVALID`.
const P_INVALID: c_int = -1;

/* ---------------------------------------------------------------------------
 * Shared accessors -- the progs_builtins_sv_fx.rs block, unchanged.
 */

#[inline]
fn sv_p() -> *mut Server {
    ptr::addr_of_mut!(sv)
}

#[inline]
fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl)
}

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

/// `progs.h` `G_FLOAT (o) = v`
#[inline]
unsafe fn set_g_float(vm: *mut QcVm, ofs: usize, v: c_float) {
    // SAFETY: as `g_float`.
    unsafe { *globals(vm).add(ofs) = v };
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
    // SAFETY: as `g_float`; the 3 floats at `ofs..ofs+3` stay in bounds for an
    // `OFS_PARM*` offset.
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
// are UB in C and saturate in Rust; the same shim progs_builtins_sv.rs uses.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/// `progs.h` `G_STRING (o)`, guarded: `PR_GetString` `Host_Error`s on a handle
/// outside the string table.
#[inline]
unsafe fn g_string(ofs: usize) -> Result<*const c_char, SvRaise> {
    let mut out: *const c_char = ptr::null();
    // SAFETY: `out` is a live slot; the seam clears it before jumping.
    unsafe { guarded(msgg::PRBI_MsgGlue_GetString(ofs as c_int, &mut out))? };
    Ok(out)
}

/// `progs.h` `NUM_FOR_EDICT (e)`, guarded (`world_glue.c`).
#[inline]
unsafe fn num_for_edict(ent: *mut Edict) -> Result<c_int, SvRaise> {
    let mut out: c_int = 0;
    // SAFETY: `ent` is a live edict pointer; `out` is a live slot.
    unsafe {
        guarded(c::world::World_Glue_NumForEdict(
            ent.cast::<c_void>(),
            &mut out,
        ))?
    };
    Ok(out)
}

/* ---------------------------------------------------------------------------
 * C string primitives. `pr_ext.c` calls `strcmp`, `strncmp` and `strstr` on
 * progs- and cvar-owned bytes; these transcribe them rather than pulling libc
 * in, since none of the three has a determinism-bearing implementation.
 */

/// `strcmp (a, b) == 0`
unsafe fn c_streq(a: *const c_char, b: *const c_char) -> bool {
    let mut i = 0isize;
    loop {
        // SAFETY: both are NUL-terminated; the scan stops at the first NUL.
        let (ca, cb) = unsafe { (*a.offset(i), *b.offset(i)) };
        if ca != cb {
            return false;
        }
        if ca == 0 {
            return true;
        }
        i += 1;
    }
}

/// `strncmp (s, lit, lit.len ()) == 0`. `lit` must contain no NUL, so a short
/// `s` mismatches at its terminator and the scan never runs past it.
unsafe fn c_starts_with(s: *const c_char, lit: &[u8]) -> bool {
    for (i, &want) in lit.iter().enumerate() {
        // SAFETY: see above.
        if unsafe { *s.add(i) } as u8 != want {
            return false;
        }
    }
    true
}

/// `strstr (hay, needle) != NULL`. `needle` must be non-empty and NUL-free.
///
/// COMPAT: `hay` is dereferenced without a null check, exactly like C's
/// `strstr (r_particledesc.string, "effectinfo")` -- a cvar's `string` is never
/// NULL once `Cvar_RegisterVariable` has run.
unsafe fn c_contains(hay: *const c_char, needle: &[u8]) -> bool {
    let mut base = 0isize;
    loop {
        let mut i = 0usize;
        while i < needle.len() {
            // SAFETY: the inner scan stops at the first mismatch, and a NUL in
            // `hay` mismatches every byte of `needle`.
            if unsafe { *hay.offset(base + i as isize) } as u8 != needle[i] {
                break;
            }
            i += 1;
        }
        if i == needle.len() {
            return true;
        }
        // SAFETY: `base` has not passed the terminator yet.
        if unsafe { *hay.offset(base) } == 0 {
            return false;
        }
        base += 1;
    }
}

/* ---------------------------------------------------------------------------
 * Buffered writer against `sv.multicast` (ADR-009 rule 3).
 *
 * The longest run in this module is 9 ops (`PF_sv_trailparticles`, and
 * `PF_sv_pointparticles`'s velocity arm); `push` still auto-flushes at capacity
 * so the bound is not load-bearing. Nothing here reads `cursize` between
 * writes, so unlike `sv_send.rs`'s `Writer` there is no accessor that has to
 * flush first.
 */

/// `svsend_write_t.kind` values -- must match `Quake/sv_send_glue.c`.
const W_BYTE: c_int = 0;
const W_SHORT: c_int = 2;
const W_STRING: c_int = 5;
const W_COORD: c_int = 6;

const WRITE_BATCH: usize = 9;

struct Multicast {
    ops: [sendg::SvSendWriteOp; WRITE_BATCH],
    n: usize,
}

impl Multicast {
    fn new() -> Self {
        Multicast {
            ops: [sendg::SvSendWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                u: 0,
                p: ptr::null(),
            }; WRITE_BATCH],
            n: 0,
        }
    }

    unsafe fn flush(&mut self) -> SvResult {
        if self.n == 0 {
            return Ok(());
        }
        let count = self.n;
        self.n = 0;
        // SAFETY: `sv.multicast` is live storage; `ops[..count]` is initialised
        // and every `p` pointer is still live at this point.
        unsafe {
            guarded(sendg::SvSend_Glue_WriteBatch(
                ptr::addr_of_mut!((*sv_p()).multicast).cast::<c_void>(),
                self.ops.as_ptr(),
                count as c_int,
            ))
        }
    }

    unsafe fn push(
        &mut self,
        kind: c_int,
        i: c_int,
        f: c_float,
        u: c_uint,
        p: *const c_void,
    ) -> SvResult {
        if self.n == WRITE_BATCH {
            // SAFETY: see `flush`.
            unsafe { self.flush()? };
        }
        self.ops[self.n] = sendg::SvSendWriteOp { kind, i, f, u, p };
        self.n += 1;
        Ok(())
    }

    unsafe fn byte(&mut self, v: c_int) -> SvResult {
        // SAFETY: see `push`.
        unsafe { self.push(W_BYTE, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn short(&mut self, v: c_int) -> SvResult {
        // SAFETY: see `push`.
        unsafe { self.push(W_SHORT, v, 0.0, 0, ptr::null()) }
    }

    /// `s` must stay live until the next flush.
    unsafe fn string(&mut self, s: *const c_char) -> SvResult {
        // SAFETY: see `push`.
        unsafe { self.push(W_STRING, 0, 0.0, 0, s.cast::<c_void>()) }
    }

    /// `MSG_WriteCoord (&sv.multicast, f, sv.protocolflags)`.
    unsafe fn coord(&mut self, f: c_float) -> SvResult {
        // SAFETY: see `push`.
        unsafe {
            let flags = (*sv_p()).protocolflags;
            self.push(W_COORD, 0, f, flags, ptr::null())
        }
    }
}

/// `SV_Multicast (to, org, 0, requireext2)`, guarded.
#[inline]
unsafe fn sv_multicast(to: c_int, org: *mut c_float, requireext2: c_uint) -> SvResult {
    // SAFETY: `org` is either NULL or a live 3-float vector in the qcvm
    // globals; the trampoline lives in `pr_ext.c` beside `SV_Multicast`.
    unsafe { guarded(extg::PRExt_Glue_SVMulticast(to, org, 0, requireext2)) }
}

/* ---------------------------------------------------------------------------
 * `pr_ext.c:52` `static int pr_ext_warned_particleeffectnum`.
 *
 * Module-private: under `-Duse_rust_host` the only readers are the two
 * `particleeffectnum` builtins here, and the only other writer is
 * `PR_RSH_ResetParticleWarnCount ()`, which routes to
 * `quake_rs_pr_reset_particle_warn_count` below. The C static stays defined --
 * the unflipped C bodies still compile -- but nothing reads it.
 */
static mut PR_EXT_WARNED_PARTICLEEFFECTNUM: c_int = 0;

/// `if (pr_ext_warned_particleeffectnum++ < 3)`.
///
/// COMPAT: ADR-010 -- plain `int` post-increment, allowed to overflow in C.
unsafe fn warn_budget_consumed() -> bool {
    // SAFETY: single-threaded progs execution, as for every other builtin
    // static in this crate.
    unsafe {
        let old = PR_EXT_WARNED_PARTICLEEFFECTNUM;
        PR_EXT_WARNED_PARTICLEEFFECTNUM = old.wrapping_add(1);
        old < 3
    }
}

/// `Con_Warning ("PF_sv_particleeffectnum(%s): Precache should only be done in
/// spawn functions\n", s)`, assembled as bytes for [`SvConsole`]'s `"%s"`.
unsafe fn warn_precache(con: &mut SvConsole, s: *const c_char) {
    let mut msg: Vec<u8> = Vec::new();
    msg.extend_from_slice(b"PF_sv_particleeffectnum(");
    let mut i = 0isize;
    loop {
        // SAFETY: `s` is a NUL-terminated progs string.
        let ch = unsafe { *s.offset(i) } as u8;
        if ch == 0 {
            break;
        }
        msg.push(ch);
        i += 1;
    }
    msg.extend_from_slice(b"): Precache should only be done in spawn functions\n");
    con.warn(&msg);
}

/* ---------------------------------------------------------------------------
 * Server builtins.
 */

/// `pr_ext.c:4626` `PF_sv_particleeffectnum`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_particleeffectnum(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, con: &mut SvConsole| -> SvResult {
            let s = g_string(OFS_PARM0)?;
            set_g_float(vm, OFS_RETURN, 0.0);

            if *s == 0 {
                return Ok(());
            }

            if (*sv_p()).particle_precache[1].is_null()
                && (c_starts_with(s, b"effectinfo.")
                    || c_contains((*ptr::addr_of!(extg::r_particledesc)).string, b"effectinfo"))
            {
                guarded(extg::PRExt_Glue_EffectinfoEnumerate())?;
            }

            for i in 1..MAX_PARTICLETYPES {
                let have = (*sv_p()).particle_precache[i];
                if !have.is_null() && c_streq(have, s) {
                    if (*sv_p()).state != SS_LOADING
                        && (*ptr::addr_of!(c::world::pr_checkextension)).value == 0.0
                        && warn_budget_consumed()
                    {
                        warn_precache(con, s);
                    }
                    set_g_float(vm, OFS_RETURN, i as c_float);
                    return Ok(());
                }
            }

            for i in 1..MAX_PARTICLETYPES {
                if (*sv_p()).particle_precache[i].is_null() {
                    if (*sv_p()).state != SS_LOADING {
                        if warn_budget_consumed() {
                            warn_precache(con, s);
                        }

                        let mut w = Multicast::new();
                        w.byte(SVCDP_PRECACHE)?;
                        w.short(i as c_int | 0x4000)?;
                        w.string(s)?;
                        w.flush()?;
                        sv_multicast(MULTICAST_ALL_R, ptr::null_mut(), PEXT2_REPLACEMENTDELTAS)?;
                    }

                    // weirdness to avoid issues with tempstrings
                    (*sv_p()).particle_precache[i] = c::cvar_cmd::q_strdup(s);
                    set_g_float(vm, OFS_RETURN, i as c_float);
                    return Ok(());
                }
            }

            Err(SvRaise {
                status: PRBI_ERR_SV_PARTICLEEFFECTNUM_OVERFLOW,
                detail: 0,
            })
        })
    }
}

/// `MAX_EDICTS * (unsigned int) qcvm->edict_size`.
///
/// COMPAT: ADR-010 -- C's unsigned multiply, allowed to wrap.
#[inline]
unsafe fn dp_compat_edict_span(vm: *mut QcVm) -> c_uint {
    // SAFETY: `vm` is the live ambient qcvm.
    unsafe { (MAX_EDICTS as c_uint).wrapping_mul((*vm).edict_size as c_uint) }
}

/// `pr_ext.c:4678` `PF_sv_trailparticles`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_trailparticles(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let start = g_vector(vm, OFS_PARM2);
            let end = g_vector(vm, OFS_PARM3);

            /*DP gets this wrong, lets try to be compatible*/
            let (efnum, ednum) = if (g_int(vm, OFS_PARM1) as c_uint) >= dp_compat_edict_span(vm) {
                let ednum = num_for_edict(g_edict(vm, OFS_PARM0))?;
                (as_int(g_float(vm, OFS_PARM1)), ednum)
            } else {
                let efnum = as_int(g_float(vm, OFS_PARM0));
                (efnum, num_for_edict(g_edict(vm, OFS_PARM1))?)
            };

            if efnum <= 0 {
                return Ok(());
            }

            let mut w = Multicast::new();
            w.byte(SVCDP_TRAILPARTICLES)?;
            w.short(ednum)?;
            w.short(efnum)?;
            for k in 0..3 {
                w.coord(*start.add(k))?;
            }
            for k in 0..3 {
                w.coord(*end.add(k))?;
            }
            w.flush()?;

            sv_multicast(MULTICAST_PHS_U, start, PEXT2_REPLACEMENTDELTAS)
        })
    }
}

/// `pr_ext.c:4708` `PF_sv_pointparticles`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_pointparticles(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let efnum = as_int(g_float(vm, OFS_PARM0));
            let org = g_vector(vm, OFS_PARM1);
            let vel: *const c_float = if (*vm).argc < 3 {
                ptr::addr_of!(crate::mathlib::vec3_origin).cast::<c_float>()
            } else {
                g_vector(vm, OFS_PARM2)
            };
            let mut count = if (*vm).argc < 4 {
                1
            } else {
                as_int(g_float(vm, OFS_PARM3))
            };

            if efnum <= 0 {
                return Ok(());
            }
            if count > 65535 {
                count = 65535;
            }
            if count < 1 {
                return Ok(());
            }

            let mut w = Multicast::new();
            if count == 1 && *vel == 0.0 && *vel.add(1) == 0.0 && *vel.add(2) == 0.0 {
                w.byte(SVCDP_POINTPARTICLES1)?;
                w.short(efnum)?;
                for k in 0..3 {
                    w.coord(*org.add(k))?;
                }
            } else {
                w.byte(SVCDP_POINTPARTICLES)?;
                w.short(efnum)?;
                for k in 0..3 {
                    w.coord(*org.add(k))?;
                }
                for k in 0..3 {
                    w.coord(*vel.add(k))?;
                }
                w.short(count)?;
            }
            w.flush()?;

            sv_multicast(MULTICAST_PVS_U, org, PEXT2_REPLACEMENTDELTAS)
        })
    }
}

/* ---------------------------------------------------------------------------
 * Client helpers. Both are non-`static` in C but have no caller outside
 * `pr_ext.c`; the C definitions stay for the switches-off build.
 */

/// `&cl.particle_precache[i]` / `&cl.local_particle_precache[i]`.
#[inline]
unsafe fn cl_precache(local: bool, i: usize) -> *mut ParticlePrecacheEntry {
    // SAFETY: every caller's `i` is inside `1..MAX_PARTICLETYPES`.
    unsafe {
        let base = if local {
            ptr::addr_of_mut!((*cl_p()).local_particle_precache)
        } else {
            ptr::addr_of_mut!((*cl_p()).particle_precache)
        };
        base.cast::<ParticlePrecacheEntry>().add(i)
    }
}

/// `pr_ext.c:4739` `PF_CL_ForceParticlePrecache`.
unsafe fn cl_force_particle_precache(s: *const c_char) -> Result<c_int, SvRaise> {
    // SAFETY: `s` is a NUL-terminated progs string; see `cl_precache`.
    unsafe {
        // check if an ssqc one already exists with that name
        for i in 1..MAX_PARTICLETYPES {
            let e = cl_precache(false, i);
            if (*e).name.is_null() {
                break; // nope, no more known
            }
            if c_streq((*e).name, s) {
                return Ok(i as c_int);
            }
        }

        // nope, check for a csqc one, and allocate if needed
        for i in 1..MAX_PARTICLETYPES {
            let e = cl_precache(true, i);
            if (*e).name.is_null() {
                // weirdness to avoid issues with tempstrings
                (*e).name = c::cvar_cmd::q_strdup(s);
                let mut idx: c_int = 0;
                guarded(clg::ClMain_Glue_FindParticleType((*e).name, &mut idx))?;
                (*e).index = idx;
                return Ok(-(i as c_int));
            }
            if c_streq((*e).name, s) {
                return Ok(-(i as c_int));
            }
        }

        // err... too many. bum.
        Ok(0)
    }
}

/// `pr_ext.c:4780` `PF_CL_GetParticle`. Negatives are csqc-originated
/// particles, positives are ssqc-originated.
///
/// COMPAT: ADR-004 -- C's `idx = -idx` on `INT_MIN` is UB that wraps back to
/// `INT_MIN` and then indexes negatively. `wrapping_neg` reproduces the wrap,
/// and the extra `idx < 0` test turns the out-of-bounds read into `P_INVALID`.
/// Unreachable from either in-file caller: both guard on `efnum > 0`.
unsafe fn cl_get_particle(idx: c_int) -> c_int {
    // SAFETY: every index below is tested against `MAX_PARTICLETYPES` first.
    unsafe {
        if idx == 0 {
            return P_INVALID;
        }
        if idx < 0 {
            let idx = idx.wrapping_neg();
            if idx < 0 || idx >= MAX_PARTICLETYPES as c_int {
                return P_INVALID;
            }
            (*cl_precache(true, idx as usize)).index
        } else {
            if idx >= MAX_PARTICLETYPES as c_int {
                return P_INVALID;
            }
            (*cl_precache(false, idx as usize)).index
        }
    }
}

/* ---------------------------------------------------------------------------
 * Client builtins.
 */

/// `pr_ext.c:4798` `PF_cl_particleeffectnum`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_particleeffectnum(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let s = g_string(OFS_PARM0)?;
            set_g_float(vm, OFS_RETURN, 0.0);

            if *s == 0 {
                return Ok(());
            }

            let idx = cl_force_particle_precache(s)?;
            set_g_float(vm, OFS_RETURN, idx as c_float);
            // C re-reads the float it just stored; exact for |idx| < 2048.
            if g_float(vm, OFS_RETURN) == 0.0 {
                return Err(SvRaise {
                    status: PRBI_ERR_CL_PARTICLEEFFECTNUM_OVERFLOW,
                    detail: 0,
                });
            }
            Ok(())
        })
    }
}

/// `pr_ext.c:4813` `PF_cl_trailparticles`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_trailparticles(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let start = g_vector(vm, OFS_PARM2);
            let end = g_vector(vm, OFS_PARM3);

            let (mut efnum, ent) = if (g_int(vm, OFS_PARM1) as c_uint) >= dp_compat_edict_span(vm) {
                /*DP gets this wrong, lets try to be compatible*/
                (as_int(g_float(vm, OFS_PARM1)), g_edict(vm, OFS_PARM0))
            } else {
                (as_int(g_float(vm, OFS_PARM0)), g_edict(vm, OFS_PARM1))
            };

            if efnum <= 0 {
                return Ok(());
            }
            efnum = cl_get_particle(efnum);

            let dlkey = num_for_edict(ent)?.wrapping_neg();
            guarded(clg::ClMain_Glue_ParticleTrail(
                start,
                end,
                efnum,
                // C passes the `double` host_frametime to a `float` parameter.
                c::host_frametime as c_float,
                dlkey,
                ptr::null(),
                ptr::null_mut(),
            ))
        })
    }
}

/// `pr_ext.c:4835` `PF_cl_pointparticles`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_pointparticles(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con: &mut SvConsole| -> SvResult {
            let mut efnum = as_int(g_float(vm, OFS_PARM0));
            let org = g_vector(vm, OFS_PARM1);
            let vel: *const c_float = if (*vm).argc < 3 {
                ptr::addr_of!(crate::mathlib::vec3_origin).cast::<c_float>()
            } else {
                g_vector(vm, OFS_PARM2)
            };
            let count = if (*vm).argc < 4 {
                1
            } else {
                as_int(g_float(vm, OFS_PARM3))
            };

            if efnum <= 0 {
                return Ok(());
            }
            if count < 1 {
                return Ok(());
            }
            efnum = cl_get_particle(efnum);

            guarded(clg::ClMain_Glue_RunParticleEffectState(
                org,
                vel,
                // C passes the `int` count to a `float` parameter.
                count as c_float,
                efnum,
                ptr::null_mut(),
            ))
        })
    }
}

/* ------------------------------------------------------------------------- */

/// `pr_ext_warned_particleeffectnum = 0`, reached through
/// `PR_RSH_ResetParticleWarnCount ()` and `rust_pr_ResetParticleWarnCount`.
///
/// # Safety
/// Called from `PR_EnableExtensions`, outside progs execution.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_reset_particle_warn_count() {
    // SAFETY: single-threaded, as for every other builtin static here.
    unsafe { PR_EXT_WARNED_PARTICLEEFFECTNUM = 0 };
}
