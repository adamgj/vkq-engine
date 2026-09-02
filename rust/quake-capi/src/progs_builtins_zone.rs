//! pr_ext.c strzone/strunzone + the knownzone bitmap (Phase 7 M9d).
//!
//! `PF_strzone`, `PF_strunzone` and `PR_UnzoneAll` (`Quake/pr_ext.c:564-635`),
//! the three functions that own the `qcvm->knownzone` bitmap. The C bodies
//! stay compiled as the oracle; the flip is Pattern C -- two `builtin_t` table
//! slots through `pr_cmds_glue.c`'s `RUST_PF` wrappers, plus a hand-written
//! frame for `PR_UnzoneAll`, which is not a builtin (its only caller is
//! `PR_ShutdownExtensions`, `pr_ext.c:6167`).
//!
//! # Shared state
//!
//! `ED_RezoneString` (`Quake/pr_edict_parse.c:65-99`) drives the same
//! `qcvm->knownzone` / `knownzonesize` fields with the same growth and clear
//! arithmetic and stays C in this milestone. Nothing here may change the
//! bitmap's representation: the growth rounding (`(id + 32) & ~7` bits, only
//! the bytes past the old size zeroed) lives in `quake_progs::arena`'s
//! `knownzone_grow_to`, which is the port of *that* C block, so both sides
//! agree by construction.
//!
//! # Why this module is `host`-gated, not `progs`-gated
//!
//! Nothing in these three functions actually needs the host stratum -- they
//! reach only `Mem_*`, `PR_GetString`, `PR_SetEngineString`,
//! `PR_ClearEngineString` and one `Con_Warning`. The gate is inherited from
//! the plumbing they reuse: `run_sv` / `SvConsole` live in
//! `progs_builtins_sv.rs`, which is `all(host, progs-host)`, and the matching
//! `RUST_PF` wrappers live in `pr_cmds_glue.c`'s `#ifdef USE_RUST_HOST` block.
//! The C table rows are therefore `PF_RSH`, so the module gate, the glue's
//! compilation condition and the table rows are identical in every
//! configuration. A `progs`-only flip would need its own frame and its own
//! `RUST_PF` block; it is not worth a second copy of the plumbing.
//!
//! # ADR-009 audit
//!
//! Only one seam here can raise, and it is reported as a status rather than
//! guarded, following `pr_cmds_glue.c`'s own convention for it:
//!
//! * `PR_GetString` (`pr_edict_arena.c:307`) `Host_Error`s on a negative
//!   handle whose known-string slot is null. `quake_progs::arena`'s
//!   `VmRaw::get_string` is the port of that function (invalid-offset arm's
//!   dead `Host_Error` included), so the check runs in Rust and the raise is
//!   re-issued by `PRBI_Raise`'s `PRBI_ERR_NO_STRING` arm, whose message is
//!   character-for-character C's. Reached from `PF_strzone`'s argument loop,
//!   `PF_strunzone`'s `G_STRING (OFS_PARM0)` and `PR_UnzoneAll`'s
//!   `PR_GetString (s)`.
//!
//! Everything else is a leaf: `Mem_Alloc` / `Mem_Realloc` / `Mem_Free` only
//! `Sys_Error` (ADR-013), `PR_SetEngineString`'s one `Host_Error` is inside an
//! `#if 0` (`pr_edict_arena.c:351-353`), `PR_ClearEngineString` is a table
//! write plus `Mem_Free`, and the single `Con_Warning` takes a constant format
//! string with no arguments that could themselves raise -- it is queued on
//! `SvConsole` and flushed by `run_sv` after the closure returns, like every
//! other console message in this module family.
//!
//! State on a raise is preserved: `PF_strzone` has mutated nothing when its
//! loop can raise, and `PR_UnzoneAll` returns immediately, leaving
//! `knownzonesize` at the value C's `longjmp` would have left it (the
//! post-decrement value) with the bitmap still allocated.
//!
//! # ADR-005 audit (float formatter)
//!
//! No float is formatted anywhere in these three functions. `PF_strzone`
//! concatenates the *bytes* its `G_STRING` arguments already point at; the QC
//! caller is what turns a float into a string (`ftos`), before the call. The
//! `%g`/`%e` panic path is not reachable from this module.
//!
//! # Bounds / panic audit (`panic = "abort"` in every profile)
//!
//! * The argument loop's C original writes into `const char *s[8]` /
//!   `size_t l[8]` indexed by `qcvm->argc`. `argc` is set by `OP_CALL0`..
//!   `OP_CALL8` only, so it is structurally within `0..=8` and the overflow is
//!   unreachable; the port collects into a `Vec` instead, which has no fixed
//!   bound and so cannot abort on a hypothetical `argc > 8` either.
//! * `id` is `size_t`, computed from `-1 - handle` in `int` and widened, so
//!   the arithmetic is `wrapping_sub` plus a sign-extending cast -- no
//!   overflow check to trip.
//! * `knownzone_pop` decrements a zero `size_t` to `SIZE_MAX`, exactly like
//!   C's `while (qcvm->knownzonesize-- > 0)`; it uses `wrapping_sub` so debug
//!   profiles do not abort on it.
//! * Every bitmap access goes through `quake_progs::arena`, which indexes the
//!   allocation with raw pointer arithmetic (no slice bound), matching C.
//! * `VmRaw::new`'s `assert!(!vm.is_null())` is the one abort left; a null
//!   ambient `qcvm` would already have crashed the C original, which
//!   dereferences it unconditionally.

use core::ffi::{c_char, c_int, CStr};

use quake_c_sys as c;
use quake_c_sys::progs_builtins_sv as g;
use quake_progs::arena::{Mem, StringError, VmRaw};
use quake_types::progs::{QcVm, OFS_PARM0, OFS_RETURN};

use crate::progs_builtins_sv::{run_sv, SvConsole, SvRaise, SvResult};

/// `pr_cmds_glue.c:38` `PRBI_ERR_NO_STRING` -- re-issued there as
/// `Host_Error ("PR_GetString: attempt to get a non-existant string %d\n")`.
const PRBI_ERR_NO_STRING: c_int = 2;

/// `PR_GetString`'s one live failure, as the status `PRBI_Raise` decodes.
fn no_string(e: StringError) -> SvRaise {
    match e {
        StringError::NonExistent(num) => SvRaise {
            status: PRBI_ERR_NO_STRING,
            detail: num,
        },
    }
}

/// The engine allocator (ADR-013), handed to the `quake_progs::arena` bitmap
/// helpers.
struct ZoneMem;

impl Mem for ZoneMem {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        // SAFETY: Mem_Alloc returns zeroed memory or aborts.
        unsafe { c::Mem_Alloc(size).cast() }
    }

    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        // SAFETY: `ptr` is null or came from this allocator.
        unsafe { c::Mem_Realloc(ptr.cast(), size).cast() }
    }

    fn free(&mut self, ptr: *mut u8) {
        // SAFETY: `ptr` is null or came from this allocator; Mem_Free
        // tolerates null, like C's SAFE_FREE.
        unsafe { c::Mem_Free(ptr.cast()) }
    }

    fn note_slot_growth(&mut self, _maxknownstrings: c_int) {
        // Unreachable: this allocator is only ever handed to
        // `knownzone_grow_to` / `knownzone_release`, which resize the bitmap
        // and never the known-string slot table. String-slot growth here goes
        // through the C `PR_SetEngineString`, which prints its own
        // `Con_DPrintf2`.
    }
}

/// `progs.h:174` `G_STRING (o)`: `PR_GetString (*(string_t *)&globals[o])`.
fn g_string(raw: &VmRaw, ofs: usize) -> Result<*const c_char, SvRaise> {
    raw.get_string(raw.g_i32(ofs)).map_err(no_string)
}

/// `strlen`, on a pointer into the engine's string arena.
fn c_strlen(s: *const c_char) -> usize {
    // SAFETY: every caller passes a `PR_GetString` result, which is either a
    // NUL-terminated slice of the progs string blob or an engine-owned C
    // string.
    unsafe { CStr::from_ptr(s) }.to_bytes().len()
}

/* ---------------------------------------------------------------------------
 * PF_strzone (pr_ext.c:564).
 */

fn pf_strzone(vm: *mut QcVm) -> SvResult {
    // SAFETY: ADR-008 -- a builtin only runs inside PR_ExecuteProgram, so the
    // ambient qcvm and its lumps are live for the whole call.
    let mut raw = unsafe { VmRaw::new(vm) };

    // COMPAT: C collects into `const char *s[8]` / `size_t l[8]` indexed by
    // `qcvm->argc`; see the module's bounds audit for why the `Vec` is
    // equivalent for every reachable `argc`.
    let mut parts: Vec<(*const c_char, usize)> = Vec::with_capacity(8);
    let mut len: usize = 0;
    let mut i: c_int = 0;
    while i < raw.argc() {
        let s = g_string(&raw, OFS_PARM0 + (i as usize) * 3)?;
        let l = c_strlen(s);
        len += l;
        parts.push((s, l));
        i += 1;
    }
    len += 1; /* for the null */

    // SAFETY: Mem_Alloc returns a `len`-byte block or aborts.
    let buf = unsafe { c::Mem_Alloc(len) }.cast::<c_char>();
    // SAFETY: `buf` is not in the progs string blob, so PR_SetEngineString
    // interns it; it cannot raise (its Host_Error is `#if 0`'d out).
    let handle = unsafe { g::PR_SetEngineString(buf) };
    raw.set_g_i32(OFS_RETURN, handle);

    // C: `size_t id = -1 - G_INT (OFS_RETURN);` -- the subtraction is `int`,
    // the widening to size_t sign-extends.
    let id = (-1i32).wrapping_sub(handle) as usize;
    raw.knownzone_grow_to(id, &mut ZoneMem);
    raw.knownzone_set(id);

    let mut p = buf;
    for &(s, l) in &parts {
        // SAFETY: `buf` was sized as the sum of these lengths plus one, and
        // the arena strings do not alias it.
        unsafe {
            core::ptr::copy_nonoverlapping(s.cast::<u8>(), p.cast::<u8>(), l);
            p = p.add(l);
        }
    }
    // SAFETY: exactly the trailing byte `len++` reserved.
    unsafe { p.write(0) };
    Ok(())
}

/// `pr_ext.c:564` `PF_strzone`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_strzone(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, |vm, _con| pf_strzone(vm)) }
}

/* ---------------------------------------------------------------------------
 * PF_strunzone (pr_ext.c:601).
 */

fn pf_strunzone(vm: *mut QcVm, con: &mut SvConsole) -> SvResult {
    // SAFETY: as pf_strzone.
    let mut raw = unsafe { VmRaw::new(vm) };

    // COMPAT: C resolves `G_STRING (OFS_PARM0)` in the declaration's
    // initializer, i.e. *before* the null-handle test below, so a corrupt
    // handle raises even for a call the test would have made a no-op.
    let zoned = g_string(&raw, OFS_PARM0)?;

    let handle = raw.g_i32(OFS_PARM0);
    if handle == 0 {
        return Ok(()); // don't bug out if they gave a null string
    }
    let id = (-1i32).wrapping_sub(handle) as usize;
    if raw.knownzone_test(id) {
        raw.knownzone_clear(id);
        // SAFETY: leaves -- a known-strings table write plus Mem_Free.
        unsafe {
            g::PR_ClearEngineString(handle);
            c::Mem_Free(zoned.cast());
        }
    } else {
        con.warn(b"PF_strunzone: string wasn't strzoned\n");
    }
    Ok(())
}

/// `pr_ext.c:601` `PF_strunzone`
///
/// # Safety
/// `detail` is `RUST_PF`'s `&detail`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_strunzone(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; ADR-008 ambient qcvm.
    unsafe { run_sv(detail, pf_strunzone) }
}

/* ---------------------------------------------------------------------------
 * PR_UnzoneAll (pr_ext.c:618) -- not a builtin; called by
 * PR_ShutdownExtensions at map end.
 */

fn pr_unzone_all(vm: *mut QcVm) -> SvResult {
    // SAFETY: PR_ShutdownExtensions runs with the qcvm it is tearing down
    // switched in, and dereferences it either way.
    let mut raw = unsafe { VmRaw::new(vm) };
    let mut mem = ZoneMem;

    while let Some(id) = raw.knownzone_pop() {
        if raw.knownzone_test_raw(id) {
            // C: `string_t s = -1 - (int)id;`
            let s = (-1i32).wrapping_sub(id as c_int);
            let ptr = raw.get_string(s).map_err(no_string)?;
            // SAFETY: leaves, as in pf_strunzone.
            unsafe {
                g::PR_ClearEngineString(s);
                c::Mem_Free(ptr.cast());
            }
        }
    }
    raw.knownzone_release(&mut mem);
    Ok(())
}

/// `pr_ext.c:618` `PR_UnzoneAll`
///
/// # Safety
/// `detail` must point at a writable `int`, as `pr_cmds_glue.c`'s
/// `rust_pr_UnzoneAll` frame passes.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_unzone_all(detail: *mut c_int) -> c_int {
    // SAFETY: caller contract; the ambient qcvm is the one being shut down.
    unsafe { run_sv(detail, |vm, _con| pr_unzone_all(vm)) }
}
