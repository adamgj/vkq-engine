//! Message QuakeC builtins (Rust migration Phase 7 M5, wave 2 Group D):
//! `PF_stuffcmd`, `PF_bprint`, `PF_sprint`, `PF_centerprint` (`Quake/pr_cmds.c`)
//! and the extended message writers `PF_WriteFloat` / `PF_WriteDouble` /
//! `PF_WriteInt` / `PF_WriteInt64` / `PF_WriteUInt64` / `PF_WriteString2`
//! (`Quake/pr_ext.c`). `"WriteUInt"`'s table slot shares `PF_WriteInt`'s
//! function pointer in C (`pr_ext.c:5675-5676`), so it is flipped to this
//! module's `quake_rs_pf_WriteInt` too -- there is no separate `WriteUInt`
//! export.
//!
//! These bodies come from `Quake/pr_cmds.c` / `Quake/pr_ext.c`, which stay
//! compiled as the oracle; the flip is Pattern C through `pr_cmds_glue.c`'s
//! `RUST_PF` wrappers, gated per `PF_RSH` (`host` feature).
//!
//! # Why this module is `host`-gated
//!
//! Every builtin here reads/writes `sv`/`svs`/`client_t` state that has no
//! ADR-011 mirror in `quake-types` (unlike `Edict`/`QcVm`/`GlobalVars`), so
//! `Quake/pr_cmds_sv_msg_glue.c` provides small non-raising accessors and
//! guarded seams, compiled only under `-Duse_rust_host` -- the same condition
//! this module's `#[cfg(feature = "host")]` gate mirrors (see wave 1's
//! `progs_builtins_sv.rs` module doc for the full rationale, which applies
//! unchanged here).
//!
//! # ADR-009 audit
//!
//! Every raise reachable from this module is caught by a `Host_Guard` in
//! `pr_cmds_sv_msg_glue.c` and reported as `PRBI_ERR_GUARD` (detail replayed
//! via `Host_Reraise`), or as one of `PRBI_ERR_WRITEDEST_NOT_CLIENT` /
//! `PRBI_ERR_WRITEDEST_BAD_DEST` (`pr_cmds_glue.c`'s shared `PRBI_Raise`
//! already carries both arms; no new status code is needed). The raising
//! seams are:
//!
//! * `PF_stuffcmd`'s `G_EDICTNUM (OFS_PARM0)` -- `NUM_FOR_EDICT`'s bounds
//!   check, via the existing guarded `World_Glue_NumForEdict`;
//! * `PF_stuffcmd`'s explicit `PR_RunError ("Parm 0 not a client")`
//!   (`PRBI_MsgGlue_StuffcmdClientCheck`);
//! * `PF_stuffcmd`'s `G_STRING (OFS_PARM1)` -- `PR_GetString`
//!   (`PRBI_MsgGlue_GetString`);
//! * `PF_bprint` / `PF_sprint` / `PF_centerprint`'s `PF_VarString` call, run
//!   whole in a guard (`PRBI_MsgGlue_VarString`) -- reimplementing its
//!   `LOC_Format`/localisation layer in Rust is out of M5's scope;
//! * `PF_sprint` / `PF_centerprint`'s `G_EDICTNUM (OFS_PARM0)`, same as
//!   `PF_stuffcmd`'s;
//! * the extended writers' reimplemented `WriteDest` dispatch: `G_EDICTNUM`
//!   of `pr_global_struct->msg_entity` for `MSG_ONE`, then
//!   `PRBI_ERR_WRITEDEST_NOT_CLIENT` / `PRBI_ERR_WRITEDEST_BAD_DEST` directly
//!   (`write_dest`, below) -- WriteDest() itself is not called from Rust;
//! * `PF_WriteString2`'s `G_STRING (OFS_PARM0)` (`PRBI_MsgGlue_GetString`
//!   again, same helper as `PF_stuffcmd`'s, different offset).
//!
//! `PF_sprint` / `PF_centerprint`'s own "tried to sprint/centerprint to a
//! non-client" check is a **soft, non-raising** `Con_Printf` + early return
//! (`pr_cmds.c:422-426`, `:452-456`) -- unlike `PF_stuffcmd`'s raising check
//! and unlike `WriteDest`'s `MSG_ONE` raising check. All three are kept
//! distinct below; do not conflate them.
//!
//! # ADR-005 audit (float formatter)
//!
//! No format specifier is ever routed through a Rust formatter in this
//! module. `PF_VarString`'s localisation pass (`LOC_Format`, placeholders,
//! `%g`/`%e`-capable) and `Host_ClientCommands` / `SV_BroadcastPrintf`'s
//! `vsnprintf` both run entirely in C, called through the guarded/leaf glue
//! above; this module only ever passes already-formatted byte strings or
//! `"%s"` through. ADR-005's `%g`/`%e` panic path is therefore not reachable
//! from any of the eleven builtins this module ports.
//!
//! # ADR-010 audit (float truncation / no reassociation)
//!
//! `write_dest`'s `dest = G_FLOAT (OFS_PARM0)` truncation and every extended
//! writer's `G_FLOAT`/`G_DOUBLE`/`G_INT`/`G_INT64`/`G_UINT64` payload read are
//! raw reinterpretations of the same `OFS_PARM0` global slot's bytes, exactly
//! like the C macros -- no arithmetic is reassociated, and `PF_WriteInt`'s
//! `int`-to-`double` conversion is left to C (`PRBI_MsgGlue_WriteIntAsDouble`)
//! so it is bit-for-bit identical to the original's implicit conversion.
//!
//! # Preserved bugs (transcribed, not fixed -- `Quake/pr_ext.c:2586-2612`)
//!
//! 1. **`PF_WriteInt` writes a double, not an int32** (`pr_ext.c:2602`):
//!    `MSG_WriteDouble (WriteDest (), G_INT (OFS_PARM0))` -- an 8-byte
//!    double-encoded value goes over the wire, not a 4-byte int. Reproduced by
//!    `PRBI_MsgGlue_WriteIntAsDouble`.
//! 2. **All six extended writers read `OFS_PARM0`, not `OFS_PARM1`**, even
//!    though `WriteDest()` (or here, `write_dest`) already consumes
//!    `OFS_PARM0` for the destination selector: the QC caller's actual second
//!    argument is never read, and the destination float's bit pattern is
//!    reinterpreted as the payload instead. Reproduced by reading every
//!    payload from `OFS_PARM0` below, with a `// COMPAT` at each site.
//! 3. **`"WriteUInt"` is `PF_WriteInt`, not a distinct function**
//!    (`pr_ext.c:5675-5676`): both bugs above apply identically. See the
//!    module doc above for the table-flip consequence.

use core::ffi::{c_char, c_double, c_float, c_int, c_longlong, c_ulonglong, c_void};

use quake_c_sys::progs_builtins_sv_msg as g;
use quake_c_sys::sv_phys as pg;
use quake_c_sys::world as wg;
use quake_types::progs::{Edict, GlobalVars, QcVm, OFS_PARM0, OFS_PARM1};

use crate::progs_builtins_sv::{
    guarded, run_sv, SvConsole, SvRaise, SvResult, PRBI_ERR_GUARD, PRBI_OK,
};

/* ---------------------------------------------------------------------------
 * Module-private helpers, duplicated from `progs_builtins_sv.rs` (not
 * `pub(crate)` there): `globals`, `gvars`, `g_float`, `g_int`, `prog_to_edict`,
 * `g_edict`, `as_int`. `g_double`/`g_int64`/`g_uint64` are new -- this module
 * is the first that needs the 8-byte `G_DOUBLE`/`G_INT64`/`G_UINT64` macros.
 */

#[inline]
unsafe fn globals(vm: *mut QcVm) -> *mut c_float {
    // SAFETY: the ambient qcvm is live for the duration of a builtin.
    unsafe { (*vm).globals }
}

#[inline]
unsafe fn gvars(vm: *mut QcVm) -> *mut GlobalVars {
    // SAFETY: `pr_global_struct` is the same storage as `qcvm->globals`.
    unsafe { (*vm).globals.cast::<GlobalVars>() }
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

/// `progs.h` `G_DOUBLE (o)` -- reinterprets 8 bytes starting at `globals[o]`,
/// overlapping the next global slot exactly like the C macro (this is what
/// makes preserved bug 2 above reproducible: `ofs` is always `OFS_PARM0`).
#[inline]
unsafe fn g_double(vm: *mut QcVm, ofs: usize) -> c_double {
    // SAFETY: as `g_float`; callers keep `ofs` + 1 inside the globals block.
    unsafe { *globals(vm).add(ofs).cast::<c_double>() }
}

/// `progs.h` `G_INT64 (o)`
#[inline]
unsafe fn g_int64(vm: *mut QcVm, ofs: usize) -> c_longlong {
    // SAFETY: as `g_double`.
    unsafe { *globals(vm).add(ofs).cast::<c_longlong>() }
}

/// `progs.h` `G_UINT64 (o)`
#[inline]
unsafe fn g_uint64(vm: *mut QcVm, ofs: usize) -> c_ulonglong {
    // SAFETY: as `g_double`.
    unsafe { *globals(vm).add(ofs).cast::<c_ulonglong>() }
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

/// `progs.h` `G_EDICTNUM (o)` = `NUM_FOR_EDICT (G_EDICT (o))`, guarded:
/// `NUM_FOR_EDICT`'s bounds check always runs (not only in debug builds) and
/// can `Host_Error` (`pr_edict.c:1076-1092`).
unsafe fn g_edictnum(vm: *mut QcVm, ofs: usize) -> Result<c_int, SvRaise> {
    // SAFETY: caller contract (ADR-008 ambient qcvm; `ofs` is a fixed OFS_*
    // slot).
    unsafe {
        let e = g_edict(vm, ofs);
        let mut num: c_int = 0;
        let status = wg::World_Glue_NumForEdict(e.cast::<c_void>(), &mut num);
        guarded(status)?;
        Ok(num)
    }
}

/* ---------------------------------------------------------------------------
 * WriteDest (pr_cmds.c:1627), reimplemented -- see the module doc's ADR-009
 * audit for why this is not a guarded call to WriteDest() itself.
 */

// COMPAT: server.h:308-313, duplicated (no ADR-011 mirror for these protocol
// constants exists yet).
const MSG_BROADCAST: c_int = 0;
const MSG_ONE: c_int = 1;
const MSG_ALL: c_int = 2;
const MSG_INIT: c_int = 3;
const MSG_EXT_MULTICAST: c_int = 4;
const MSG_EXT_ENTITY: c_int = 5;

/// `pr_cmds_glue.c:353` `PRBI_ERR_WRITEDEST_NOT_CLIENT` -- already wired into
/// the shared `PRBI_Raise` (`pr_cmds_glue.c:327-328`); reused, not new.
const PRBI_ERR_WRITEDEST_NOT_CLIENT: c_int = 5;
/// `pr_cmds_glue.c:353` `PRBI_ERR_WRITEDEST_BAD_DEST` -- likewise reused.
const PRBI_ERR_WRITEDEST_BAD_DEST: c_int = 6;

/// `pr_cmds.c:1627` `WriteDest`. Returns `(dest, entnum)`; `entnum` is
/// meaningful only when `dest == MSG_ONE` (the extended writers' glue indexes
/// `svs.clients` with it for that case only, mirroring `PRBI_WriteDest`'s
/// duplicated switch in both `pr_cmds_glue.c` and this module's own glue).
unsafe fn write_dest(vm: *mut QcVm) -> Result<(c_int, c_int), SvRaise> {
    // SAFETY: caller contract (ADR-008 ambient qcvm).
    unsafe {
        // COMPAT: `dest = G_FLOAT (OFS_PARM0);` -- float truncated to int
        // exactly like the C assignment (ADR-010).
        let dest = as_int(g_float(vm, OFS_PARM0));
        match dest {
            MSG_ONE => {
                let msg_entity = (*gvars(vm)).msg_entity;
                let ent = prog_to_edict(vm, msg_entity);
                let mut entnum: c_int = 0;
                let status = wg::World_Glue_NumForEdict(ent.cast::<c_void>(), &mut entnum);
                guarded(status)?;
                if entnum < 1 || entnum > pg::SvPhys_Glue_MaxClients() {
                    return Err(SvRaise {
                        status: PRBI_ERR_WRITEDEST_NOT_CLIENT,
                        detail: 0,
                    });
                }
                Ok((dest, entnum))
            }
            MSG_BROADCAST | MSG_ALL | MSG_INIT | MSG_EXT_MULTICAST | MSG_EXT_ENTITY => {
                Ok((dest, 0))
            }
            _ => Err(SvRaise {
                status: PRBI_ERR_WRITEDEST_BAD_DEST,
                detail: 0,
            }),
        }
    }
}

/* ---------------------------------------------------------------------------
 * PF_stuffcmd (pr_cmds.c:931).
 */

/// `pr_cmds.c:931` `PF_stuffcmd`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_stuffcmd(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let entnum = g_edictnum(vm, OFS_PARM0)?;
            guarded(g::PRBI_MsgGlue_StuffcmdClientCheck(entnum))?;
            let mut str_: *const c_char = core::ptr::null();
            guarded(g::PRBI_MsgGlue_GetString(OFS_PARM1 as c_int, &mut str_))?;
            g::PRBI_MsgGlue_ClientCommandsPlain(entnum, str_);
            Ok(())
        })
    }
}

/* ---------------------------------------------------------------------------
 * PF_bprint / PF_sprint / PF_centerprint (pr_cmds.c:396, :413, :443).
 */

/// `pr_cmds.c:396` `PF_bprint`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_bprint(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |_vm, _con| {
            let mut out = [0 as c_char; 1024];
            guarded(g::PRBI_MsgGlue_VarString(0, out.as_mut_ptr()))?;
            g::PRBI_MsgGlue_BroadcastPrintfPlain(out.as_ptr());
            Ok(())
        })
    }
}

/// `pr_cmds.c:413` `PF_sprint`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sprint(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, con| sprint_or_centerprint(vm, con, 0)) }
}

/// `pr_cmds.c:443` `PF_centerprint`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_centerprint(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, con| sprint_or_centerprint(vm, con, 1)) }
}

/// Shared body for `PF_sprint` (`kind` 0) and `PF_centerprint` (`kind` 1):
/// identical except for the `svc_print` / `svc_centerprint` byte
/// (`pr_cmds.c:413-432`, `:443-462`).
///
/// COMPAT: `entnum = G_EDICTNUM (OFS_PARM0); s = PF_VarString (1);` runs in
/// that order in C, and *both* execute before the "not a client" check --
/// `PF_VarString`'s guard/overflow warnings fire even when `entnum` is bad.
/// This is preserved by resolving `entnum` and `s` before the range check
/// below, same as the four-line body it mirrors.
unsafe fn sprint_or_centerprint(vm: *mut QcVm, con: &mut SvConsole, kind: c_int) -> SvResult {
    // SAFETY: caller contract (ADR-008 ambient qcvm).
    unsafe {
        let entnum = g_edictnum(vm, OFS_PARM0)?;
        let mut out = [0 as c_char; 1024];
        guarded(g::PRBI_MsgGlue_VarString(1, out.as_mut_ptr()))?;

        if !(1..=pg::SvPhys_Glue_MaxClients()).contains(&entnum) {
            // pr_cmds.c:422-426 / :452-456 -- a soft warning, NOT a raise
            // (unlike PF_stuffcmd's and WriteDest's "not a client" checks).
            // COMPAT: preserved bug -- PF_centerprint's warning text is
            // copy-pasted from PF_sprint's ("tried to sprint...", not
            // "tried to centerprint..."); both branches read the SAME
            // literal in C, so `kind` does not select the message.
            con.print(b"tried to sprint to a non-client\n");
            return Ok(());
        }

        g::PRBI_MsgGlue_ClientMessageWrite(entnum, kind, out.as_ptr());
        Ok(())
    }
}

/* ---------------------------------------------------------------------------
 * Extended message writers (pr_ext.c:2587-2611).
 */

/// `pr_ext.c:2592` `PF_WriteFloat`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_WriteFloat(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let (dest, entnum) = write_dest(vm)?;
            // COMPAT: preserved bug 2 -- reads OFS_PARM0, not OFS_PARM1.
            let f = g_float(vm, OFS_PARM0);
            g::PRBI_MsgGlue_WriteFloat(dest, entnum, f);
            Ok(())
        })
    }
}

/// `pr_ext.c:2596` `PF_WriteDouble`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_WriteDouble(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let (dest, entnum) = write_dest(vm)?;
            // COMPAT: preserved bug 2 -- reads OFS_PARM0, not OFS_PARM1.
            let f = g_double(vm, OFS_PARM0);
            g::PRBI_MsgGlue_WriteDouble(dest, entnum, f);
            Ok(())
        })
    }
}

/// `pr_ext.c:2600` `PF_WriteInt`. Also serves `"WriteUInt"`'s table slot
/// (`pr_ext.c:5676`), which points at the same C function -- see the module
/// doc.
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_WriteInt(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let (dest, entnum) = write_dest(vm)?;
            // COMPAT: preserved bug 2 -- reads OFS_PARM0, not OFS_PARM1.
            let v = g_int(vm, OFS_PARM0);
            // COMPAT: preserved bug 1 -- writes a double, not an int32
            // (PRBI_MsgGlue_WriteIntAsDouble calls MSG_WriteDouble).
            g::PRBI_MsgGlue_WriteIntAsDouble(dest, entnum, v);
            Ok(())
        })
    }
}

/// `pr_ext.c:2604` `PF_WriteInt64`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_WriteInt64(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let (dest, entnum) = write_dest(vm)?;
            // COMPAT: preserved bug 2 -- reads OFS_PARM0, not OFS_PARM1.
            let v = g_int64(vm, OFS_PARM0);
            g::PRBI_MsgGlue_WriteInt64(dest, entnum, v);
            Ok(())
        })
    }
}

/// `pr_ext.c:2608` `PF_WriteUInt64`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_WriteUInt64(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let (dest, entnum) = write_dest(vm)?;
            // COMPAT: preserved bug 2 -- reads OFS_PARM0, not OFS_PARM1.
            let v = g_uint64(vm, OFS_PARM0);
            g::PRBI_MsgGlue_WriteUInt64(dest, entnum, v);
            Ok(())
        })
    }
}

/// `pr_ext.c:2587` `PF_WriteString2`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_WriteString2(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe {
        run_sv(detail, |vm, _con| {
            let (dest, entnum) = write_dest(vm)?;
            let mut string: *const c_char = core::ptr::null();
            // COMPAT: preserved bug 2 -- reads OFS_PARM0, not OFS_PARM1.
            guarded(g::PRBI_MsgGlue_GetString(OFS_PARM0 as c_int, &mut string))?;
            g::PRBI_MsgGlue_WriteString2(dest, entnum, string);
            Ok(())
        })
    }
}

// Silence "unused" for items only referenced via re-export symmetry with
// progs_builtins_sv.rs's own constants; kept for readers cross-checking
// against pr_cmds_glue.c's PRBI_Raise switch.
#[allow(dead_code)]
const _PRBI_OK_ECHO: c_int = PRBI_OK;
#[allow(dead_code)]
const _PRBI_ERR_GUARD_ECHO: c_int = PRBI_ERR_GUARD;
