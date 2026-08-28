//! C-ABI shim for the savegame writer (`pr_edict_save.c` → `quake-progs`).
//!
//! `Quake/pr_edict_save_glue.c` owns the `FILE *` writes and the one raise.
//! The byte buffers here are reused across calls and stay valid until the next
//! call, which is what the glue's `fwrite`-then-return pattern needs.

use core::ffi::{c_char, c_int};

use quake_c_sys as c;
use quake_progs::arena::VmRaw;
use quake_progs::save::{self, value_words, SaveError, SaveSys};
use quake_types::progs::{DDef, QcVm};

/// Status codes shared with `Quake/pr_edict_save_glue.c` (keep in sync).
const PRSAVE_OK: c_int = 0;
const PRSAVE_ERR_NO_STRING: c_int = 1;
const PRSAVE_ERR_BAD_EDICT: c_int = 2;

/// Reused output buffers. Progs writing is single-threaded main-thread work
/// (savegames are written from `host_cmd.c`), and each buffer is consumed by
/// the glue's `fwrite` before the next call can start.
static mut ED_WRITE_BUF: Vec<u8> = Vec::new();
static mut ED_GLOBALS_BUF: Vec<u8> = Vec::new();

struct EngineSave;

impl SaveSys for EngineSave {
    fn field_at_ofs(&mut self, ofs: c_int) -> Option<DDef> {
        let (mut ty, mut field_ofs, mut s_name) = (0, 0, 0);
        // SAFETY: a leaf lookup in pr_edict.c over the loaded fielddefs.
        let found = unsafe { c::PRSave_Glue_FieldAtOfs(ofs, &mut ty, &mut field_ofs, &mut s_name) };
        (found != 0).then_some(DDef {
            type_: ty as u16,
            ofs: field_ofs as u16,
            s_name,
        })
    }
}

fn encode(err: SaveError, detail: &mut c_int) -> c_int {
    match err {
        SaveError::NonExistentString(n) => {
            *detail = n;
            PRSAVE_ERR_NO_STRING
        }
        SaveError::BadEdictPointer => {
            *detail = 0;
            PRSAVE_ERR_BAD_EDICT
        }
    }
}

/// # Safety
///
/// The ambient `qcvm` must be selected and loaded.
unsafe fn ambient_vm() -> VmRaw {
    // SAFETY: the ambient qcvm is the VM the host frame selected, and the
    // mirror is verified by quake-ctest/tests/progs_abi.rs.
    unsafe { VmRaw::new(c::qcvm.cast::<QcVm>()) }
}

/// `PR_UglyValueString`, rendered into the caller's static buffer.
///
/// COMPAT: C uses `q_snprintf` into a 1024-byte buffer, so an over-long value
/// is truncated rather than growing; the same cap applies here.
///
/// # Safety
///
/// `val` must point at enough words for `ty`; `out` must be `out_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_ugly_value_string(
    ty: c_int,
    val: *const i32,
    out: *mut c_char,
    out_len: usize,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns both.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    // Exactly the words this type occupies. C reads 1-3 words depending on
    // the arm, and the callers (pr_edict.c's ED_Print, pr_ext.c's
    // putentityfieldstring/pr_dumpplatform) pass pointers into the globals
    // block or an edict's field block -- so a fixed 4-word slice would run
    // past the allocation for a def at either tail, which
    // `slice::from_raw_parts` forbids even without a read.
    // SAFETY: `val` addresses at least `value_words(ty)` words, which is what
    // the type occupies and all C reads.
    let words = unsafe { core::slice::from_raw_parts(val, value_words(ty)) };

    // SAFETY: see ambient_vm.
    let vm = unsafe { ambient_vm() };
    let mut sys = EngineSave;
    match save::ugly_value_string(&vm, &mut sys, ty, words) {
        Ok(bytes) => {
            let n = bytes.len().min(out_len.saturating_sub(1));
            // SAFETY: n < out_len, and the buffers do not overlap.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), n);
                out.add(n).write(0);
            }
            PRSAVE_OK
        }
        Err(e) => encode(e, detail),
    }
}

/// `ED_Write`, as a byte buffer for the glue to `fwrite`.
///
/// # Safety
///
/// Called only from `pr_edict_save_glue.c` with the ambient `qcvm` loaded.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_write(
    num: c_int,
    out: *mut *const u8,
    out_len: *mut usize,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns all three.
    let (out, out_len, detail) = unsafe { (&mut *out, &mut *out_len, &mut *detail) };
    *detail = 0;

    // SAFETY: single-threaded main-thread use; the previous buffer has been
    // consumed by the glue's fwrite before this call.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(ED_WRITE_BUF) };
    buf.clear();

    // SAFETY: see ambient_vm.
    let vm = unsafe { ambient_vm() };
    let mut sys = EngineSave;
    match save::ed_write(&vm, &mut sys, num, buf) {
        Ok(()) => {
            *out = buf.as_ptr();
            *out_len = buf.len();
            PRSAVE_OK
        }
        Err(e) => encode(e, detail),
    }
}

/// `ED_WriteGlobals`, as a byte buffer for the glue to `fwrite`.
///
/// # Safety
///
/// Called only from `pr_edict_save_glue.c` with the ambient `qcvm` loaded.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_write_globals(
    out: *mut *const u8,
    out_len: *mut usize,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns all three.
    let (out, out_len, detail) = unsafe { (&mut *out, &mut *out_len, &mut *detail) };
    *detail = 0;

    // SAFETY: as in quake_rs_ed_write.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(ED_GLOBALS_BUF) };
    buf.clear();

    // SAFETY: see ambient_vm.
    let vm = unsafe { ambient_vm() };
    let mut sys = EngineSave;
    match save::ed_write_globals(&vm, &mut sys, buf) {
        Ok(()) => {
            *out = buf.as_ptr();
            *out_len = buf.len();
            PRSAVE_OK
        }
        Err(e) => encode(e, detail),
    }
}
