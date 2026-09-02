//! `pr_ext.c` temp-entity group (Phase 7 M9f group D): the sv/cl
//! `PF_*_te_*` builtins (`Quake/pr_ext.c:2647-3061`).
//!
//! 33 real function bodies are ported here: the 3 `SV_StartParticle`-based
//! builtins (`sv_te_blooddp`/`bloodqw`/`lightningblood`), 16 `PF_sv_te_*`
//! network writers (`spike`, `superspike`, `gunshot`, `explosion`,
//! `tarexplosion`, `lightning1`, `lightning2`, `wizspike`, `knightspike`,
//! `lightning3`, `lavasplash`, `teleport`, `explosion2`, `beam`,
//! `particlerain`, `particlesnow`), and 14 `PF_cl_te_*` client-side visual/
//! audio effects (the same list minus `particlerain`/`particlesnow`, which
//! have no client builtin). The 12 `#define` aliases at `pr_ext.c:3049-3060`
//! (`sv_te_gunshotquad` etc.) are not separate definitions -- 5 alias an
//! in-range function directly and 7 alias the shared `PF_void_stub`, so none
//! of them need their own `quake_rs_pf_*` export; they resolve at the C
//! table-flip level once the corresponding `RUST_PF` invocations exist.
//!
//! These bodies come from `Quake/pr_ext.c`, which stays compiled as the
//! oracle; the flip is Pattern C, gated by `PF_RSH` (both `-Duse_rust_progs`
//! and `-Duse_rust_host`), same as `progs_builtins_sv_msg`/`_sv_fx`/`_cl`.
//!
//! # Naming: `quake_rs_pf_<name>`, not `rust_pf_<name>`
//!
//! Every sibling module in this crate exports `quake_rs_pf_<name>(detail:
//! *mut c_int) -> c_int` and relies on a `RUST_PF(<name>)` invocation in
//! `Quake/pr_cmds_glue.c` to generate the `rust_pf_<name>(void)` C trampoline
//! that calls it and replays any raise via `PRBI_Raise` *after* the Rust
//! frame has returned (ADR-009 rule 3: the jump then crosses zero Rust
//! frames). This module follows that same convention rather than exporting
//! bare `rust_pf_<name>(void)` symbols directly, because several of these
//! builtins **can** raise (`NUM_FOR_EDICT`, `Mod_ForName`, `SZ_GetSpace`
//! overflow) and a bare `extern "C" fn rust_pf_<name>()` that called
//! `PRBI_Raise` itself would raise *from inside* its own Rust frame --
//! exactly the crossing ADR-009 forbids. `Quake/pr_cmds_glue.c` is out of
//! this group's edit scope, so the 33 `RUST_PF(<name>)` invocations that
//! would generate the production `rust_pf_<name>` trampolines are not added
//! here; see the M9f group D report for the exact list to add.
//!
//! # ADR-009 audit
//!
//! Every raise reachable from this module is caught by a `Host_Guard` and
//! reported as `PRBI_ERR_GUARD` with the guard status as `detail`. The
//! raising seams are:
//!
//! * `lightning1`/`lightning2`/`lightning3`/`beam` (both `sv_te_*` and
//!   `cl_te_*`): `G_EDICT (OFS_PARM0)`'s `NUM_FOR_EDICT`, via
//!   `World_Glue_NumForEdict` (`world.rs`);
//! * every `MSG_Write*` call: `SZ_GetSpace`'s overflow `Host_Error`, via
//!   `SvMain_Glue_WriteBatch` / `SvSend_Glue_WriteBatch`;
//! * `cl_te_lightning1`/`lightning2`/`lightning3`/`beam`'s `Mod_ForName
//!   (name, true)`, via `ClTent_Glue_ModForName` (`cl_tent.rs`) -- the
//!   `crash=true` path `Host_Error`s when the model is missing.
//!
//! # ADR-006 (`SV_Multicast` / `PF_multicast_internal`)
//!
//! `SV_Multicast` (`pr_ext.c:4216`) and `PF_multicast_internal`
//! (`pr_ext.c:4169`) are both `static`, so they have no external linkage and
//! cannot be called from Rust; their logic is reimplemented in
//! [`sv_multicast_unreliable`] below, restricted to the three
//! `multicast_t` values this group's call sites actually use (confirmed by
//! grepping every `SV_Multicast (` call in `pr_ext.c`):
//!
//! * `MULTICAST_PVS_U` (`spike`/`superspike`/`gunshot`, 3 sites) -- real
//!   per-client PVS fanout into each visible client's `.datagram`, using the
//!   same `Mod_PointInLeaf`/`Mod_LeafPVS`/cluster-from-pointer-difference
//!   technique as `quake_rs_pf_sv_checkclient` (`progs_builtins_sv.rs`).
//! * `MULTICAST_PHS_U` (the other 11 `sv_datagram`/`sv_multicast` writers)
//!   -- `PF_multicast_internal`'s `!pvs && !requireext2` branch, since this
//!   engine never computes a real PHS (`pvs` is always `NULL` at every
//!   `MULTICAST_PHS_*` call site in `SV_Multicast`, "we don't support phs").
//!   This collapses to a single `SZ_Write` of `sv.multicast`'s current bytes
//!   into `sv.datagram`.
//! * `MULTICAST_ALL_U` with `requireext2 = PEXT2_REPLACEMENTDELTAS`
//!   (`particlerain`/`particlesnow`, 2 sites) -- the `!pvs && requireext2`
//!   branch: per active client gated by `protocol_pext2 & requireext2`,
//!   fanned into each such client's `.datagram`.
//!
//! `MULTICAST_ONE_*`/`MULTICAST_INIT` are never used by this group and are
//! not implemented. Every call site here passes the `_U` (unreliable) enum
//! value, so `reliable` is not threaded through -- every write targets
//! `.datagram`, never `.message`. [`sv_multicast_unreliable`] also
//! reproduces `SV_Multicast`'s trailing `SZ_Clear (&sv.multicast)`
//! unconditionally, exactly like the C.
//!
//! # ADR-010 audit (float truncation / no reassociation)
//!
//! * `SV_StartParticle`'s wire encoding is already ported
//!   (`quake_rs_sv_start_particle`, `sv_main.rs`) and reused directly here
//!   for `blooddp`/`bloodqw`/`lightningblood` -- no float logic is
//!   duplicated.
//! * `particlerain`/`particlesnow`'s `count`/`colour` are read as `G_FLOAT`
//!   and passed to `MSG_WriteShort`/`MSG_WriteByte`, an implicit
//!   float-to-int conversion at the call site; reproduced with the same
//!   `as_int` saturating shim `progs_builtins_sv.rs`/`_sv_msg.rs` use
//!   (`// COMPAT` at each site).
//! * `explosion2`'s `palstart`/`palcount` (`sv` side) and `colorStart`/
//!   `colorLength` (`cl` side) both read `G_FLOAT (OFS_PARM1)` -- **not**
//!   `OFS_PARM2` for the second value. This is a preserved bug (transcribed
//!   below, not fixed): the "count" parameter is never actually read; both
//!   locals get the same value as the "start" parameter.
//!
//! # Preserved bugs (transcribed, not fixed)
//!
//! 1. **`PF_sv_te_explosion2`/`PF_cl_te_explosion2` read the same global
//!    twice** (`pr_ext.c:2935-2936`, `:2949-2950`): `palcount`/`colorLength`
//!    read `OFS_PARM1` again instead of `OFS_PARM2`, so the QC caller's third
//!    argument is never observed by either side.
//! 2. **`SV_Multicast`'s trailing call after `spike`/`superspike`/`gunshot`/
//!    the 11 `PHS_U` writers is effectively a no-op in the common case**:
//!    each of those builtins writes its own payload straight to
//!    `sv.datagram` (already unconditionally broadcast to every connected
//!    client later that frame), not to `sv.multicast`, so unless another
//!    builtin left bytes in `sv.multicast` earlier in the same server frame,
//!    the multicast fanout copies zero bytes. Reproduced faithfully (not
//!    special-cased away) since a same-frame ordering dependency on
//!    `sv.multicast` is possible in principle.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys::cl_tent as clt;
use quake_c_sys::progs_builtins_cl as clg;
use quake_c_sys::progs_builtins_sv as svg;
use quake_c_sys::sv_main as smg;
use quake_c_sys::sv_send as ssg;
use quake_c_sys::world as wg;
use quake_c_sys::COM_Rand;
use quake_types::host::ClientState;
use quake_types::model_mem::{MLeaf, QModel};
use quake_types::progs::{Edict, QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_PARM3, OFS_PARM4};

use crate::progs_builtins_sv::{guarded, run_sv, SvRaise, SvResult};
use crate::sv_main::{sv, svs};

/* ---------------------------------------------------------------------------
 * progs.h macro equivalents, duplicated locally (private in `progs_builtins_
 * sv.rs`, matching every wave-2 module's own convention).
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

/// `progs.h` `PROG_TO_EDICT (e)` -- byte offset, no bounds check.
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

/// `progs.h` `G_EDICTNUM (o)` = `NUM_FOR_EDICT (G_EDICT (o))`, guarded.
unsafe fn g_edictnum(vm: *mut QcVm, ofs: usize) -> Result<c_int, SvRaise> {
    // SAFETY: caller contract (ADR-008 ambient qcvm; `ofs` a fixed OFS_* slot).
    unsafe {
        let e = g_edict(vm, ofs);
        let mut num: c_int = 0;
        guarded(wg::World_Glue_NumForEdict(e.cast::<c_void>(), &mut num))?;
        Ok(num)
    }
}

// COMPAT: ADR-010 -- C's implicit float->int conversion. Out-of-range values
// are UB in C and saturate in Rust; the same shim every sibling module uses.
#[inline]
fn as_int(x: c_float) -> c_int {
    x as c_int
}

/* ---------------------------------------------------------------------------
 * `cl` (`client_state_t`), needed for `cl.time` (dlight `die`). Re-declared
 * here rather than imported from `crate::cl_main::cl`, matching
 * `cl_tent.rs`/`cl_input.rs`/`sv_main.rs`'s own per-module extern.
 */
extern "C" {
    static mut cl: ClientState;
}

extern "C" {
    /// `Quake/client.h:393` -- `void CL_UpdateBeam (struct qmodel_s *m, const
    /// char *trailname, const char *impactname, int ent, float *start,
    /// float *end);` (defined `Quake/cl_tent.c:59`). Non-`static`,
    /// externally linked; not yet bound anywhere in `quake-c-sys`, so
    /// declared here directly rather than adding a new C file.
    fn CL_UpdateBeam(
        m: *mut c_void,
        trailname: *const c_char,
        impactname: *const c_char,
        ent: c_int,
        start: *mut c_float,
        end: *mut c_float,
    );
}

/* ---------------------------------------------------------------------------
 * Protocol constants (`Quake/protocol.h`), duplicated locally -- no ADR-011
 * mirror exists for these, matching every sibling module's own convention.
 */

const SVC_TEMP_ENTITY: c_int = 23;
const TE_SPIKE: c_int = 0;
const TE_SUPERSPIKE: c_int = 1;
const TE_GUNSHOT: c_int = 2;
const TE_EXPLOSION: c_int = 3;
const TE_TAREXPLOSION: c_int = 4;
const TE_LIGHTNING1: c_int = 5;
const TE_LIGHTNING2: c_int = 6;
const TE_WIZSPIKE: c_int = 7;
const TE_KNIGHTSPIKE: c_int = 8;
const TE_LIGHTNING3: c_int = 9;
const TE_LAVASPLASH: c_int = 10;
const TE_TELEPORT: c_int = 11;
const TE_EXPLOSION2: c_int = 12;
const TE_BEAM: c_int = 13;
const TEDP_PARTICLERAIN: c_int = 55;
const TEDP_PARTICLESNOW: c_int = 56;
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;

/* ---------------------------------------------------------------------------
 * Batched `MSG_Write*` against an arbitrary `sizebuf_t *`, mirroring
 * `sv_main.rs`'s module-private `Writer` (not `pub(crate)` there, so
 * duplicated here exactly like `progs_builtins_sv_msg.rs` duplicates its own
 * copy of `progs_builtins_sv.rs`'s helpers).
 */

const WRITE_BATCH: usize = 16;
const W_BYTE: c_int = 0;
const W_SHORT: c_int = 2;
const W_COORD: c_int = 5;

struct Writer {
    sb: *mut c_void,
    ops: [smg::SvMainWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new(sb: *mut c_void) -> Self {
        Writer {
            sb,
            ops: [smg::SvMainWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                s: ptr::null(),
            }; WRITE_BATCH],
            n: 0,
        }
    }

    unsafe fn push(&mut self, kind: c_int, i: c_int, f: c_float) -> SvResult {
        if self.n == WRITE_BATCH {
            // SAFETY: see `flush`.
            unsafe { self.flush()? };
        }
        self.ops[self.n] = smg::SvMainWriteOp {
            kind,
            i,
            f,
            s: ptr::null(),
        };
        self.n += 1;
        Ok(())
    }

    unsafe fn byte(&mut self, v: c_int) -> SvResult {
        // SAFETY: see `push`.
        unsafe { self.push(W_BYTE, v, 0.0) }
    }

    unsafe fn short(&mut self, v: c_int) -> SvResult {
        // SAFETY: see `push`.
        unsafe { self.push(W_SHORT, v, 0.0) }
    }

    /// `MSG_WriteCoord (sb, f, sv.protocolflags)` -- the flags argument is
    /// read inside the glue.
    unsafe fn coord(&mut self, f: c_float) -> SvResult {
        // SAFETY: see `push`.
        unsafe { self.push(W_COORD, 0, f) }
    }

    unsafe fn flush(&mut self) -> SvResult {
        if self.n == 0 {
            return Ok(());
        }
        let count = self.n;
        self.n = 0;
        // SAFETY: `sb` points at a live `sizebuf_t`; `ops[..count]` is
        // initialised.
        guarded(unsafe { smg::SvMain_Glue_WriteBatch(self.sb, self.ops.as_ptr(), count as c_int) })
    }
}

/* ---------------------------------------------------------------------------
 * `SV_Multicast` / `PF_multicast_internal` reimplementation -- see the module
 * doc's ADR-006 section.
 */

/// Copies `sv.multicast`'s current bytes into `dest` via one `SZ_Write`
/// through `SvSend_Glue_WriteBatch` (kind 10). Mirrors `SZ_Write (dest,
/// sv.multicast.data, sv.multicast.cursize)` at every
/// `PF_multicast_internal` call site (`pr_ext.c:4177`, `:4186`, `:4208`,
/// `:4210`).
unsafe fn sz_write_multicast(dest: *mut c_void) -> SvResult {
    // SAFETY: `dest` points at a live `sizebuf_t`; `sv.multicast.data` stays
    // valid for the duration of this single guarded call.
    unsafe {
        if sv.multicast.cursize == 0 {
            return Ok(());
        }
        let op = ssg::SvSendWriteOp {
            kind: 10, // SZ_Write
            i: sv.multicast.cursize,
            f: 0.0,
            u: 0,
            p: sv.multicast.data.cast::<c_void>(),
        };
        guarded(ssg::SvSend_Glue_WriteBatch(dest, &op, 1))
    }
}

/// `PF_multicast_internal` (`pr_ext.c:4169-4214`) plus `SV_Multicast`'s
/// trailing `SZ_Clear (&sv.multicast)` (`pr_ext.c:4259`). Every Group D call
/// site passes the `_U` (unreliable) enum value, so `reliable` is always
/// `false` and not threaded through.
///
/// `pvs`: null for the "no PVS" branch (`MULTICAST_PHS_U`, which collapses to
/// broadcast since PHS is unsupported; or `MULTICAST_ALL_U`, gated only by
/// `requireext2`); non-null for the real `MULTICAST_PVS_U` fanout.
unsafe fn sv_multicast_unreliable(vm: *mut QcVm, pvs: *const u8, requireext2: c_uint) -> SvResult {
    // SAFETY: caller contract; ADR-008 ambient qcvm; `pvs`, when non-null,
    // covers every cluster of `(*vm).worldmodel`.
    unsafe {
        if pvs.is_null() {
            if requireext2 == 0 {
                sz_write_multicast(ptr::addr_of_mut!(sv.datagram).cast())?;
            } else {
                let mut i: c_int = 0;
                while i < svs.maxclients {
                    let client = svs.clients.offset(i as isize);
                    if (*client).active && (*client).protocol_pext2 & requireext2 != 0 {
                        sz_write_multicast(ptr::addr_of_mut!((*client).datagram).cast())?;
                    }
                    i += 1;
                }
            }
        } else {
            let leafs = (*(*vm).worldmodel.cast::<QModel>()).leafs;
            let mut i: c_int = 0;
            while i < svs.maxclients {
                let client = svs.clients.offset(i as isize);
                if !(*client).active {
                    i += 1;
                    continue;
                }
                if requireext2 != 0 && (*client).protocol_pext2 & requireext2 == 0 {
                    i += 1;
                    continue;
                }
                let playerleaf = svg::Mod_PointInLeaf(
                    ptr::addr_of_mut!((*(*client).edict).v.origin).cast(),
                    (*vm).worldmodel,
                );
                // COMPAT: ADR-006 -- C's `(leaf - worldmodel->leafs) - 1` is a
                // raw pointer subtraction with no null check; spelled as an
                // address difference so a NULL leaf is not Rust UB, exactly
                // like `quake_rs_pf_sv_checkclient` (`progs_builtins_sv.rs`).
                let cluster = ((playerleaf as isize - leafs as isize)
                    / core::mem::size_of::<MLeaf>() as isize)
                    as c_int
                    - 1;
                if cluster < 0
                    || (*pvs.offset((cluster >> 3) as isize) & (1u8 << (cluster & 7))) != 0
                {
                    sz_write_multicast(ptr::addr_of_mut!((*client).datagram).cast())?;
                }
                i += 1;
            }
        }
        ssg::SvSend_Glue_SzClear(ptr::addr_of_mut!(sv.multicast).cast());
        Ok(())
    }
}

/// `MULTICAST_PVS_U` (`spike`/`superspike`/`gunshot`): resolves the PVS for
/// `org`'s leaf, exactly like `SV_Multicast`'s `MULTICAST_PVS_*` case.
unsafe fn sv_multicast_pvs_u(vm: *mut QcVm, org: *mut c_float) -> SvResult {
    // SAFETY: caller contract; `org` points at 3 floats.
    unsafe {
        let leaf = svg::Mod_PointInLeaf(org, (*vm).worldmodel);
        let pvs = svg::Mod_LeafPVS(leaf, (*vm).worldmodel);
        sv_multicast_unreliable(vm, pvs, 0)
    }
}

/// `MULTICAST_PHS_U` (the 11 remaining `sv_te_*` datagram/multicast
/// writers): always `pvs = NULL`, `requireext2 = 0`.
unsafe fn sv_multicast_phs_u(vm: *mut QcVm) -> SvResult {
    // SAFETY: caller contract.
    unsafe { sv_multicast_unreliable(vm, ptr::null(), 0) }
}

/// `MULTICAST_ALL_U` with `requireext2 = PEXT2_REPLACEMENTDELTAS`
/// (`particlerain`/`particlesnow`).
unsafe fn sv_multicast_all_u_ext2(vm: *mut QcVm) -> SvResult {
    // SAFETY: caller contract.
    unsafe { sv_multicast_unreliable(vm, ptr::null(), PEXT2_REPLACEMENTDELTAS) }
}

/* ---------------------------------------------------------------------------
 * `PF_sv_te_blooddp` / `PF_sv_te_bloodqw` / `PF_sv_te_lightningblood`
 * (`pr_ext.c:2647-2670`) -- all three defer straight to `SV_StartParticle`,
 * already ported as `quake_rs_sv_start_particle` (`sv_main.rs`); no new wire
 * logic is written here.
 */

/// `pr_ext.c:2647` `PF_sv_te_blooddp`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_blooddp(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            let dir = g_vector(vm, OFS_PARM1);
            let count = g_float(vm, OFS_PARM2);
            guarded(crate::sv_main::quake_rs_sv_start_particle(
                org,
                dir,
                73,
                count as c_int,
            ))
        })
    }
}

/// `pr_ext.c:2655` `PF_sv_te_bloodqw`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_bloodqw(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            let mut dir: [c_float; 3] = [0.0; 3];
            // COMPAT: ADR-010 -- `G_FLOAT (OFS_PARM1) * 20` truncated to int
            // by `SV_StartParticle`'s own `count` parameter type.
            let count = g_float(vm, OFS_PARM1) * 20.0;
            guarded(crate::sv_main::quake_rs_sv_start_particle(
                org,
                dir.as_mut_ptr(),
                73,
                count as c_int,
            ))
        })
    }
}

/// `pr_ext.c:2663` `PF_sv_te_lightningblood`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_lightningblood(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            let mut dir: [c_float; 3] = [0.0, 0.0, -100.0];
            guarded(crate::sv_main::quake_rs_sv_start_particle(
                org,
                dir.as_mut_ptr(),
                20,
                225,
            ))
        })
    }
}

/* ---------------------------------------------------------------------------
 * `sv_te_*` network writers with a single `org` vector and a PVS fanout
 * (`spike`/`superspike`/`gunshot`, `pr_ext.c:2671-2740`).
 */

unsafe fn te_org_pvs(vm: *mut QcVm, te_type: c_int) -> SvResult {
    // SAFETY: caller contract.
    unsafe {
        let org = g_vector(vm, OFS_PARM0);
        let mut w = Writer::new(ptr::addr_of_mut!(sv.datagram).cast());
        w.byte(SVC_TEMP_ENTITY)?;
        w.byte(te_type)?;
        w.coord(*org.add(0))?;
        w.coord(*org.add(1))?;
        w.coord(*org.add(2))?;
        w.flush()?;
        sv_multicast_pvs_u(vm, org)
    }
}

/// `pr_ext.c:2671` `PF_sv_te_spike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_spike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_pvs(vm, TE_SPIKE)) }
}

/// `pr_ext.c:2700` `PF_sv_te_superspike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_superspike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_pvs(vm, TE_SUPERSPIKE)) }
}

/// `pr_ext.c:2730` `PF_sv_te_gunshot`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_gunshot(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_pvs(vm, TE_GUNSHOT)) }
}

/* ---------------------------------------------------------------------------
 * `sv_te_*` network writers with a single `org` vector and a PHS_U fanout
 * (`explosion`/`tarexplosion`/`wizspike`/`knightspike`/`lavasplash`,
 * `pr_ext.c:2749-2907`).
 */

unsafe fn te_org_phs(vm: *mut QcVm, te_type: c_int) -> SvResult {
    // SAFETY: caller contract.
    unsafe {
        let org = g_vector(vm, OFS_PARM0);
        let mut w = Writer::new(ptr::addr_of_mut!(sv.datagram).cast());
        w.byte(SVC_TEMP_ENTITY)?;
        w.byte(te_type)?;
        w.coord(*org.add(0))?;
        w.coord(*org.add(1))?;
        w.coord(*org.add(2))?;
        w.flush()?;
        sv_multicast_phs_u(vm)
    }
}

/// `pr_ext.c:2749` `PF_sv_te_explosion`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_explosion(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_phs(vm, TE_EXPLOSION)) }
}

/// `pr_ext.c:2773` `PF_sv_te_tarexplosion`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_tarexplosion(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_phs(vm, TE_TAREXPLOSION)) }
}

/// `pr_ext.c:2839` `PF_sv_te_wizspike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_wizspike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_phs(vm, TE_WIZSPIKE)) }
}

/// `pr_ext.c:2857` `PF_sv_te_knightspike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_knightspike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_phs(vm, TE_KNIGHTSPIKE)) }
}

/// `pr_ext.c:2899` `PF_sv_te_lavasplash`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_lavasplash(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_org_phs(vm, TE_LAVASPLASH)) }
}

/* ---------------------------------------------------------------------------
 * `sv_te_lightning{1,2,3}`: edict num + start + end into `sv.datagram`, PHS_U
 * fanout (`pr_ext.c:2791-2897` odd entries).
 */

unsafe fn te_lightning_datagram(vm: *mut QcVm, te_type: c_int) -> SvResult {
    // SAFETY: caller contract.
    unsafe {
        let num = g_edictnum(vm, OFS_PARM0)?;
        let start = g_vector(vm, OFS_PARM1);
        let end = g_vector(vm, OFS_PARM2);
        let mut w = Writer::new(ptr::addr_of_mut!(sv.datagram).cast());
        w.byte(SVC_TEMP_ENTITY)?;
        w.byte(te_type)?;
        w.short(num)?;
        w.coord(*start.add(0))?;
        w.coord(*start.add(1))?;
        w.coord(*start.add(2))?;
        w.coord(*end.add(0))?;
        w.coord(*end.add(1))?;
        w.coord(*end.add(2))?;
        w.flush()?;
        sv_multicast_phs_u(vm)
    }
}

/// `pr_ext.c:2791` `PF_sv_te_lightning1`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_lightning1(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_lightning_datagram(vm, TE_LIGHTNING1)) }
}

/// `pr_ext.c:2815` `PF_sv_te_lightning2`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_lightning2(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_lightning_datagram(vm, TE_LIGHTNING2)) }
}

/// `pr_ext.c:2875` `PF_sv_te_lightning3`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_lightning3(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| te_lightning_datagram(vm, TE_LIGHTNING3)) }
}

/* ---------------------------------------------------------------------------
 * `sv_te_teleport` / `sv_te_beam`: same shapes as above but written into
 * `sv.multicast`, then PHS_U fanout (`pr_ext.c:2916-2931`, `:2962-2977`).
 */

/// `pr_ext.c:2916` `PF_sv_te_teleport`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_teleport(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            let mut w = Writer::new(ptr::addr_of_mut!(sv.multicast).cast());
            w.byte(SVC_TEMP_ENTITY)?;
            w.byte(TE_TELEPORT)?;
            w.coord(*org.add(0))?;
            w.coord(*org.add(1))?;
            w.coord(*org.add(2))?;
            w.flush()?;
            sv_multicast_phs_u(vm)
        })
    }
}

/// `pr_ext.c:2962` `PF_sv_te_beam`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_beam(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let num = g_edictnum(vm, OFS_PARM0)?;
            let start = g_vector(vm, OFS_PARM1);
            let end = g_vector(vm, OFS_PARM2);
            let mut w = Writer::new(ptr::addr_of_mut!(sv.multicast).cast());
            w.byte(SVC_TEMP_ENTITY)?;
            w.byte(TE_BEAM)?;
            w.short(num)?;
            w.coord(*start.add(0))?;
            w.coord(*start.add(1))?;
            w.coord(*start.add(2))?;
            w.coord(*end.add(0))?;
            w.coord(*end.add(1))?;
            w.coord(*end.add(2))?;
            w.flush()?;
            sv_multicast_phs_u(vm)
        })
    }
}

/// `pr_ext.c:2932` `PF_sv_te_explosion2`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_explosion2(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let org = g_vector(vm, OFS_PARM0);
            // COMPAT: preserved bug -- `palcount` reads OFS_PARM1 again, not
            // OFS_PARM2 (see module doc).
            let palstart = as_int(g_float(vm, OFS_PARM1));
            let palcount = as_int(g_float(vm, OFS_PARM1));
            let mut w = Writer::new(ptr::addr_of_mut!(sv.multicast).cast());
            w.byte(SVC_TEMP_ENTITY)?;
            w.byte(TE_EXPLOSION2)?;
            w.coord(*org.add(0))?;
            w.coord(*org.add(1))?;
            w.coord(*org.add(2))?;
            w.byte(palstart)?;
            w.byte(palcount)?;
            w.flush()?;
            sv_multicast_phs_u(vm)
        })
    }
}

/* ---------------------------------------------------------------------------
 * `sv_te_particlerain` / `sv_te_particlesnow` (`pr_ext.c:2987-3044`) --
 * always compiled (`PSET_SCRIPT` is unconditionally defined,
 * `quakedef.h:38`).
 */

unsafe fn te_particle_weather(vm: *mut QcVm, te_type: c_int) -> SvResult {
    // SAFETY: caller contract.
    unsafe {
        let min = g_vector(vm, OFS_PARM0);
        let max = g_vector(vm, OFS_PARM1);
        let velocity = g_vector(vm, OFS_PARM2);
        let mut count = g_float(vm, OFS_PARM3);
        let colour = g_float(vm, OFS_PARM4);

        if count < 1.0 {
            return Ok(());
        }
        if count > 65535.0 {
            count = 65535.0;
        }

        let mut w = Writer::new(ptr::addr_of_mut!(sv.multicast).cast());
        w.byte(SVC_TEMP_ENTITY)?;
        w.byte(te_type)?;
        w.coord(*min.add(0))?;
        w.coord(*min.add(1))?;
        w.coord(*min.add(2))?;
        w.coord(*max.add(0))?;
        w.coord(*max.add(1))?;
        w.coord(*max.add(2))?;
        w.coord(*velocity.add(0))?;
        w.coord(*velocity.add(1))?;
        w.coord(*velocity.add(2))?;
        // COMPAT: ADR-010 -- `MSG_WriteShort`/`MSG_WriteByte` implicitly
        // truncate these floats to int at the call site.
        w.short(as_int(count))?;
        w.byte(as_int(colour))?;
        w.flush()?;
        sv_multicast_all_u_ext2(vm)
    }
}

/// `pr_ext.c:2987` `PF_sv_te_particlerain`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_particlerain(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            te_particle_weather(vm, TEDP_PARTICLERAIN)
        })
    }
}

/// `pr_ext.c:3016` `PF_sv_te_particlesnow`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_te_particlesnow(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            te_particle_weather(vm, TEDP_PARTICLESNOW)
        })
    }
}

/* ---------------------------------------------------------------------------
 * `cl_te_*` client-side visual/audio effects (`pr_ext.c:2681-2984`). None of
 * `S_PrecacheSound`/`S_StartSound`/`PScript_RunParticleEffectTypeString`/
 * `R_RunParticleEffect`/`R_ParticleExplosion(2)`/`R_BlobExplosion`/
 * `R_LavaSplash`/`R_TeleportSplash`/`CL_AllocDlight`/`COM_Rand` can raise
 * (module docs in `progs_builtins_cl.rs`/`cl_tent.rs`); only `Mod_ForName`
 * (via `ClTent_Glue_ModForName`) and `NUM_FOR_EDICT` can.
 */

unsafe fn tink_or_ric(pos: *mut c_float) {
    // SAFETY: caller contract; `pos` points at 3 floats, matching every
    // `S_StartSound` call site in this group.
    unsafe {
        if COM_Rand() % 5 != 0 {
            let sfx = clg::S_PrecacheSound(c"weapons/tink1.wav".as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
        } else {
            let rnd = COM_Rand() & 3;
            let name: &core::ffi::CStr = if rnd == 1 {
                c"weapons/ric1.wav"
            } else if rnd == 2 {
                c"weapons/ric2.wav"
            } else {
                c"weapons/ric3.wav"
            };
            let sfx = clg::S_PrecacheSound(name.as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
        }
    }
}

/// `pr_ext.c:2681` `PF_cl_te_spike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_spike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            let mut zero: [c_float; 3] = [0.0; 3];
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_SPIKE".as_ptr(),
            ) != 0
            {
                clg::R_RunParticleEffect(pos, zero.as_mut_ptr(), 0, 10);
            }
            tink_or_ric(pos);
            Ok(())
        })
    }
}

/// `pr_ext.c:2700` `PF_cl_te_superspike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_superspike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            let mut zero: [c_float; 3] = [0.0; 3];
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_SUPERSPIKE".as_ptr(),
            ) != 0
            {
                clg::R_RunParticleEffect(pos, zero.as_mut_ptr(), 0, 20);
            }
            tink_or_ric(pos);
            Ok(())
        })
    }
}

/// `pr_ext.c:2741` `PF_cl_te_gunshot`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_gunshot(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            let mut zero: [c_float; 3] = [0.0; 3];
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                20.0,
                c"TE_GUNSHOT".as_ptr(),
            ) != 0
            {
                clg::R_RunParticleEffect(pos, zero.as_mut_ptr(), 0, 20);
            }
            Ok(())
        })
    }
}

/// `pr_ext.c:2759` `PF_cl_te_explosion`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_explosion(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_EXPLOSION".as_ptr(),
            ) != 0
            {
                clt::R_ParticleExplosion(pos);
            }
            let dl = clt::CL_AllocDlight(0);
            (*dl).origin = [*pos.add(0), *pos.add(1), *pos.add(2)];
            (*dl).radius = 350.0;
            (*dl).die = (cl.time + 0.5) as c_float;
            (*dl).decay = 300.0;
            let sfx = clg::S_PrecacheSound(c"weapons/r_exp3.wav".as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
            Ok(())
        })
    }
}

/// `pr_ext.c:2773` `PF_cl_te_tarexplosion`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_tarexplosion(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_TAREXPLOSION".as_ptr(),
            ) != 0
            {
                clt::R_BlobExplosion(pos);
            }
            let sfx = clg::S_PrecacheSound(c"weapons/r_exp3.wav".as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
            Ok(())
        })
    }
}

unsafe fn cl_lightning(
    vm: *mut QcVm,
    model_name: &core::ffi::CStr,
    trailname: &core::ffi::CStr,
    impactname: &core::ffi::CStr,
) -> SvResult {
    // SAFETY: caller contract.
    unsafe {
        let num = g_edictnum(vm, OFS_PARM0)?;
        let start = g_vector(vm, OFS_PARM1);
        let end = g_vector(vm, OFS_PARM2);
        let mut model: *mut c_void = ptr::null_mut();
        guarded(clt::ClTent_Glue_ModForName(model_name.as_ptr(), &mut model))?;
        CL_UpdateBeam(
            model,
            trailname.as_ptr(),
            impactname.as_ptr(),
            -num,
            start,
            end,
        );
        Ok(())
    }
}

/// `pr_ext.c:2807` `PF_cl_te_lightning1`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_lightning1(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            cl_lightning(
                vm,
                c"progs/bolt.mdl",
                c"TE_LIGHTNING1",
                c"TE_LIGHTNING1_END",
            )
        })
    }
}

/// `pr_ext.c:2831` `PF_cl_te_lightning2`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_lightning2(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            cl_lightning(
                vm,
                c"progs/bolt2.mdl",
                c"TE_LIGHTNING2",
                c"TE_LIGHTNING2_END",
            )
        })
    }
}

/// `pr_ext.c:2891` `PF_cl_te_lightning3`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_lightning3(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            cl_lightning(
                vm,
                c"progs/bolt3.mdl",
                c"TE_LIGHTNING3",
                c"TE_LIGHTNING3_END",
            )
        })
    }
}

/// `pr_ext.c:2978` `PF_cl_te_beam`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_beam(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            cl_lightning(vm, c"progs/beam.mdl", c"TE_BEAM", c"TE_BEAM_END")
        })
    }
}

/// `pr_ext.c:2849` `PF_cl_te_wizspike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_wizspike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            let mut zero: [c_float; 3] = [0.0; 3];
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_WIZSPIKE".as_ptr(),
            ) != 0
            {
                clg::R_RunParticleEffect(pos, zero.as_mut_ptr(), 20, 30);
            }
            let sfx = clg::S_PrecacheSound(c"wizard/hit.wav".as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
            Ok(())
        })
    }
}

/// `pr_ext.c:2867` `PF_cl_te_knightspike`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_knightspike(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            let mut zero: [c_float; 3] = [0.0; 3];
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_KNIGHTSPIKE".as_ptr(),
            ) != 0
            {
                clg::R_RunParticleEffect(pos, zero.as_mut_ptr(), 226, 20);
            }
            let sfx = clg::S_PrecacheSound(c"hknight/hit.wav".as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
            Ok(())
        })
    }
}

/// `pr_ext.c:2909` `PF_cl_te_lavasplash`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_lavasplash(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_LAVASPLASH".as_ptr(),
            ) != 0
            {
                clt::R_LavaSplash(pos);
            }
            Ok(())
        })
    }
}

/// `pr_ext.c:2926` `PF_cl_te_teleport`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_teleport(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            if clg::PScript_RunParticleEffectTypeString(
                pos,
                ptr::null_mut(),
                1.0,
                c"TE_TELEPORT".as_ptr(),
            ) != 0
            {
                clt::R_TeleportSplash(pos);
            }
            Ok(())
        })
    }
}

/// `pr_ext.c:2946` `PF_cl_te_explosion2`.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cl_te_explosion2(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let pos = g_vector(vm, OFS_PARM0);
            // COMPAT: preserved bug -- `colorLength` reads OFS_PARM1 again,
            // not OFS_PARM2 (see module doc, same bug as the `sv` side).
            let color_start = as_int(g_float(vm, OFS_PARM1));
            let color_length = as_int(g_float(vm, OFS_PARM1));
            let name = clt::ClTent_Glue_Explosion2Name(color_start, color_length);
            if clg::PScript_RunParticleEffectTypeString(pos, ptr::null_mut(), 1.0, name) != 0 {
                clt::R_ParticleExplosion2(pos, color_start, color_length);
            }
            let dl = clt::CL_AllocDlight(0);
            (*dl).origin = [*pos.add(0), *pos.add(1), *pos.add(2)];
            (*dl).radius = 350.0;
            (*dl).die = (cl.time + 0.5) as c_float;
            (*dl).decay = 300.0;
            let sfx = clg::S_PrecacheSound(c"weapons/r_exp3.wav".as_ptr());
            clg::S_StartSound(-1, 0, sfx, pos, 1.0, 1.0);
            Ok(())
        })
    }
}
