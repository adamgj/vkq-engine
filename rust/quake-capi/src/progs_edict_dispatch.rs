//! C-ABI shim for the two "key dispatchers" Phase 6 deliberately left in C
//! (`Quake/pr_edict.c` `ED_ParseGlobals`/`ED_ParseEdict`, Rust migration
//! Phase 7 M5 T5.2).
//!
//! `Quake/pr_edict_dispatch_glue.c` owns the two hash lookups
//! (`ED_FindGlobal`/`ED_FindField`) and the `sv.state` read that have no
//! Rust-visible representation. Everything else -- the value parser itself,
//! and its one raise -- routes through `crate::progs_parse`'s already-ported
//! `quake_rs_ed_parse_epair`/`quake_rs_ed_new_string` (ADR-009: these are the
//! Rust *cores*, not the C `ED_ParseEpair`/`ED_NewString` re-raising
//! wrappers in `pr_edict_parse_glue.c`, which must never be called from a
//! Rust frame).
//!
//! Status codes are shared with the `#ifdef USE_RUST_HOST` rewrite of
//! `Quake/pr_edict.c` (keep in sync; see the T5.2 manifest). `PRPARSE_*`
//! below duplicates `crate::progs_parse`'s private constants of the same
//! name rather than importing them: both mirror `Quake/pr_edict_parse_glue.c`,
//! an existing cross-language contract, so a local copy carries no more risk
//! than the import and avoids widening that module's visibility.

use core::ffi::{c_char, c_int, c_ushort, CStr};
use std::ffi::CString;

use quake_c_sys as c;
use quake_c_sys::progs_edict_dispatch as ed;
use quake_progs::alloc;
use quake_progs::arena::{EdictArena, EdictId, VmRaw, EDICT_V_OFFSET};
use quake_types::progs::QcVm;

use crate::progs_parse::{quake_rs_ed_new_string, quake_rs_ed_parse_epair};

/// Status codes shared with `Quake/pr_edict.c`'s `USE_RUST_HOST` rewrite of
/// `ED_ParseGlobals`/`ED_ParseEdict` (keep in sync -- see the T5.2 manifest).
const PREDD_OK: c_int = 0;
/// "... : EOF without closing brace"
const PREDD_ERR_EOF: c_int = 1;
/// "... : closing brace without data"
const PREDD_ERR_CLOSE_NO_DATA: c_int = 2;
/// "ED_ParseGlobals: parse error" / "ED_ParseEdict: parse error"
const PREDD_ERR_EPAIR_PARSE: c_int = 3;
/// Pass-through from `ED_ParseEpair`; `*detail` is the offending entity
/// number.
const PREDD_ERR_ENTITY_RANGE: c_int = 4;
/// Pass-through; `*detail` is the offending (negative) entity number.
const PREDD_ERR_BAD_EDICT_NUM: c_int = 5;
/// Pass-through; `*detail` is unused (matches `pr_edict_parse_glue.c`).
const PREDD_ERR_FREELIST_FULL: c_int = 6;
/// Pass-through; `*detail` is `max_edicts`.
const PREDD_ERR_FREELIST_OVER_MAX: c_int = 7;
/// A `Host_Guard`-wrapped call (`PREdictDispatch_Glue_PrecacheModel`) caught
/// a longjmp; `*detail` is the raw `HOST_GUARD_*` value for the C wrapper's
/// `Host_Reraise` (mirrors `pr_cmds_glue.c`'s `PRBI_ERR_GUARD`).
const PREDD_ERR_GUARD: c_int = 8;

/// `crate::progs_parse`'s private status constants, duplicated here (see the
/// module doc comment). Must stay in sync with `pr_edict_parse_glue.c`.
const PRPARSE_OK: c_int = 0;
const PRPARSE_FALSE: c_int = 1;
const PRPARSE_ERR_ENTITY_RANGE: c_int = 2;
const PRPARSE_ERR_BAD_EDICT_NUM: c_int = 3;
const PRPARSE_ERR_FREELIST_FULL: c_int = 4;
const PRPARSE_ERR_FREELIST_OVER_MAX: c_int = 5;

/// `Quake/common.h` `cpe_mode`.
const CPE_NOTRUNC: c_int = 0;
const CPE_ALLOWTRUNC: c_int = 1;

/// Concatenate the pieces of a console message. Kept as raw bytes, not
/// `String`: Quake strings routinely carry high-bit bytes (the coloured-text
/// charset), and a lossy UTF-8 round trip would print different bytes than
/// C's `%s`.
fn join(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum::<usize>() + 1);
    for p in parts {
        out.extend(p.iter().copied().filter(|&b| b != 0));
    }
    out
}

/// Console output accumulated rather than printed inline: `Con_Printf`/
/// `Con_DPrintf` are not leaves (they can reach `SCR_UpdateScreen`), so they
/// must not run while a parse loop holds its raw view of the VM/arena.
/// `true` selects `Con_DPrintf`, `false` selects `Con_Printf`.
type Pending = Vec<(bool, Vec<u8>)>;

fn flush(pending: &mut Pending) {
    for (is_dprint, msg) in pending.drain(..) {
        let Ok(c_msg) = CString::new(msg) else {
            continue;
        };
        // SAFETY: "%s" plus a NUL-terminated argument; no VM/arena view is
        // live here -- flush() is only ever called after the loop exits.
        unsafe {
            if is_dprint {
                c::Con_DPrintf(c"%s".as_ptr(), c_msg.as_ptr());
            } else {
                c::Con_Printf(c"%s".as_ptr(), c_msg.as_ptr());
            }
        }
    }
}

/// `q_strlcpy (dst, src, cap)`'s truncation, without the copy: at most
/// `cap - 1` bytes of `src`.
fn truncate_token(src: &[u8], cap: usize) -> Vec<u8> {
    let n = src.len().min(cap.saturating_sub(1));
    src[..n].to_vec()
}

/// # Safety
///
/// The ambient `qcvm` must be selected.
unsafe fn ambient_vm() -> VmRaw {
    // SAFETY: the ambient qcvm is the VM the host frame selected.
    unsafe { VmRaw::new(c::qcvm.cast::<QcVm>()) }
}

/// An arena over the ambient VM's edict array. See
/// `quake-capi::progs_parse::ambient_arena` -- duplicated here rather than
/// imported because that helper is private to its module.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded (`ED_ParseGlobals`/`ED_ParseEdict` only
/// run after `PR_LoadProgs`).
unsafe fn ambient_arena(vm: &VmRaw) -> EdictArena {
    let stride = vm.edict_stride() as usize;
    let count = vm.max_edicts().max(0) as usize;
    // SAFETY: the edict array is `max_edicts * edict_size` bytes, allocated
    // by PR_LoadProgs and live for the VM's lifetime.
    unsafe { EdictArena::borrowed(vm.edicts_base(), stride, count) }
}

/// Reads the thread-local `com_token` (`Quake/common.h`) as an owned byte
/// buffer, without the NUL terminator.
///
/// # Safety
///
/// Must only be called right after a `COM_Parse`/`COM_ParseEx` call, before
/// anything else on this thread can overwrite the buffer.
unsafe fn com_token_bytes() -> Vec<u8> {
    // SAFETY: COM_ThreadToken always returns this thread's `com_token`,
    // which COM_Parse/COM_ParseEx always leave NUL-terminated.
    let p = unsafe { c::COM_ThreadToken() };
    // SAFETY: as above -- `p` is the NUL-terminated `com_token` buffer.
    unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
}

/// A `ddef_t`'s three fields, out of `PREdictDispatch_Glue_FindGlobal`/
/// `FindField`.
struct Key {
    type_: u16,
    ofs: u16,
    s_name: c_int,
}

/// # Safety
///
/// The ambient `qcvm` must be loaded.
unsafe fn find_global(name: &CStr) -> Option<Key> {
    let mut type_: c_ushort = 0;
    let mut ofs: c_ushort = 0;
    let mut s_name: c_int = 0;
    // SAFETY: three valid out-params on the stack.
    let found = unsafe {
        ed::PREdictDispatch_Glue_FindGlobal(name.as_ptr(), &mut type_, &mut ofs, &mut s_name)
    };
    found.then_some(Key { type_, ofs, s_name })
}

/// # Safety
///
/// The ambient `qcvm` must be loaded.
unsafe fn find_field(name: &CStr) -> Option<Key> {
    let mut type_: c_ushort = 0;
    let mut ofs: c_ushort = 0;
    let mut s_name: c_int = 0;
    // SAFETY: as above.
    let found = unsafe {
        ed::PREdictDispatch_Glue_FindField(name.as_ptr(), &mut type_, &mut ofs, &mut s_name)
    };
    found.then_some(Key { type_, ofs, s_name })
}

/// `Q_rint` (`Quake/q_minmax.h`), transliterated on `f64` -- C evaluates
/// `ENTALPHA_ENCODE`'s argument in `double` (`atof` returns `double`; the
/// `254.0f` literal promotes to `double` in that expression, exactly, since
/// 254 is representable in both). ADR-010: comparisons and casts only, no
/// `f32::`/`f64::` methods.
fn q_rint(x: f64) -> i32 {
    if x > 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// `CLAMP` (`Quake/q_minmax.h`), transliterated on `f64`.
fn clamp_f64(minval: f64, x: f64, maxval: f64) -> f64 {
    if x < minval {
        minval
    } else if x > maxval {
        maxval
    } else {
        x
    }
}

/// `ENTALPHA_ENCODE` (`Quake/protocol.h:222`); `ENTALPHA_DEFAULT` is `0`
/// (`Quake/protocol.h:218`).
fn entalpha_encode(a: f64) -> u8 {
    if a == 0.0 {
        0
    } else {
        q_rint(clamp_f64(1.0, a * 254.0 + 1.0, 255.0)) as u8
    }
}

/// `ED_ParseGlobals` (`Quake/pr_edict.c:745`).
///
/// # Safety
///
/// `data` must be NUL-terminated (or null), `out_data` and `detail` must be
/// valid for a single write, and the ambient `qcvm` must be loaded.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_parse_globals(
    data: *const c_char,
    out_data: *mut *const c_char,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns these.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    let mut data = data;
    let mut pending: Pending = Vec::new();

    let status = 'outer: loop {
        // SAFETY: `data` is either the caller's initial pointer or a value
        // COM_Parse itself just returned; both are valid COM_Parse inputs.
        data = unsafe { c::COM_Parse(data) };
        // SAFETY: COM_Parse always leaves com_token NUL-terminated, even on
        // EOF (it becomes empty).
        let key_tok = unsafe { com_token_bytes() };
        if key_tok.first() == Some(&b'}') {
            break PREDD_OK;
        }
        if data.is_null() {
            break PREDD_ERR_EOF;
        }
        // COMPAT: this raise's C message says "ED_ParseEntity", not
        // "ED_ParseGlobals" -- a pre-existing typo (`Quake/pr_edict.c:754`),
        // preserved by the manifest's Host_Error text, not reproduced here
        // since this function never formats the message itself.
        let keyname = truncate_token(&key_tok, 64);

        // SAFETY: as above.
        data = unsafe { c::COM_Parse(data) };
        if data.is_null() {
            break PREDD_ERR_EOF;
        }
        // SAFETY: as above.
        let value_tok = unsafe { com_token_bytes() };
        if value_tok.first() == Some(&b'}') {
            break PREDD_ERR_CLOSE_NO_DATA;
        }

        let keyname_c = CString::new(keyname.clone()).unwrap_or_default();
        // SAFETY: the ambient qcvm is loaded (precondition).
        let Some(key) = (unsafe { find_global(&keyname_c) }) else {
            pending.push((false, join(&[b"'", &keyname, b"' is not a global\n"])));
            continue;
        };

        let value_c = CString::new(value_tok).unwrap_or_default();
        // SAFETY: the ambient qcvm is loaded; `globals` is what the C caller
        // passes as `(void *) qcvm->globals`.
        let globals = unsafe { (*c::qcvm.cast::<QcVm>()).globals.cast::<c_int>() };
        let mut epair_detail: c_int = 0;
        // SAFETY: `globals + key.ofs` stays inside the globals block for
        // every def the loader hands back; `value_c` is NUL-terminated;
        // `zoned = false` matches C's `ED_ParseEpair (qcvm->globals, key,
        // com_token, false)`.
        let epair_status = unsafe {
            quake_rs_ed_parse_epair(
                globals,
                key.type_,
                key.ofs,
                key.s_name,
                value_c.as_ptr(),
                false,
                &mut epair_detail,
            )
        };
        match epair_status {
            PRPARSE_OK => continue,
            PRPARSE_FALSE => break 'outer PREDD_ERR_EPAIR_PARSE,
            PRPARSE_ERR_ENTITY_RANGE => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_ENTITY_RANGE;
            }
            PRPARSE_ERR_BAD_EDICT_NUM => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_BAD_EDICT_NUM;
            }
            PRPARSE_ERR_FREELIST_FULL => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_FREELIST_FULL;
            }
            PRPARSE_ERR_FREELIST_OVER_MAX => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_FREELIST_OVER_MAX;
            }
            _ => break 'outer PREDD_ERR_EPAIR_PARSE,
        }
    };

    // SAFETY: caller's frame owns it.
    unsafe { *out_data = data };
    flush(&mut pending);
    status
}

/// `ED_ParseEdict` (`Quake/pr_edict.c:793`).
///
/// # Safety
///
/// `data` must be NUL-terminated (or null), `edict_num` must be a valid
/// index below `qcvm->max_edicts` for an edict the caller already owns (the
/// C caller passes `NUM_FOR_EDICT_NO_CHECK (ent)`, matching
/// `PRParse_Glue_UnlinkEdict`'s established unchecked-conversion precedent:
/// `ent` is always a pointer the engine itself just produced), `out_data`
/// and `detail` must be valid for a single write, and the ambient `qcvm`
/// must be loaded.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_parse_edict(
    data: *const c_char,
    edict_num: c_int,
    out_data: *mut *const c_char,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns these.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    let mut data = data;
    let mut pending: Pending = Vec::new();
    let mut init = false;

    // SAFETY: precondition.
    let vm = unsafe { ambient_vm() };
    // SAFETY: precondition; ED_ParseEdict only runs after PR_LoadProgs.
    let mut arena = unsafe { ambient_arena(&vm) };
    let id = EdictId(edict_num as u32);

    // hack, this way never clear edict 0 = world
    if edict_num != 0 {
        // SAFETY: `progs` is populated by PR_LoadProgs before any
        // ED_ParseEdict call.
        let entityfields = unsafe { (*(*vm.as_ptr()).progs).entityfields };
        arena.clear_fields(id, entityfields);
    }

    let status = 'outer: loop {
        // SAFETY: `data` is either the caller's initial pointer or a value
        // COM_Parse/COM_ParseEx just returned.
        data = unsafe { c::COM_Parse(data) };
        // SAFETY: this COM_Parse call just returned, and nothing else on
        // this thread has touched `com_token` since.
        let mut key_tok = unsafe { com_token_bytes() };
        if key_tok.first() == Some(&b'}') {
            break PREDD_OK;
        }
        if data.is_null() {
            break PREDD_ERR_EOF;
        }

        let anglehack = if key_tok.as_slice() == b"angle" {
            key_tok = b"angles".to_vec();
            true
        } else {
            false
        };
        if key_tok.as_slice() == b"light" {
            key_tok = b"light_lev".to_vec();
        }

        let mut keyname = truncate_token(&key_tok, 256);
        while keyname.last() == Some(&b' ') {
            keyname.pop();
        }

        let mode = if keyname.as_slice() == b"wad" {
            CPE_ALLOWTRUNC
        } else {
            CPE_NOTRUNC
        };
        // SAFETY: as above.
        data = unsafe { ed::COM_ParseEx(data, mode) };
        if data.is_null() {
            break PREDD_ERR_EOF;
        }
        // SAFETY: as above -- this COM_ParseEx call just returned.
        let mut value_tok = unsafe { com_token_bytes() };
        if value_tok.first() == Some(&b'}') {
            break PREDD_ERR_CLOSE_NO_DATA;
        }

        init = true;

        if keyname.first() == Some(&b'_') {
            // SAFETY: matches C's `qcvm == &sv.qcvm` test; already
            // FFI-bound (`Quake/pr_edict_load_glue.c`).
            if unsafe { c::PRLoad_Glue_IsServerVM(c::qcvm) } {
                // SAFETY: precondition.
                let loading = unsafe { ed::PREdictDispatch_Glue_ServerLoading() };
                if loading && keyname.as_slice() == b"_precache_model" {
                    let value_c = CString::new(value_tok.clone()).unwrap_or_default();
                    // SAFETY: `value_c` is NUL-terminated.
                    let handle = unsafe { quake_rs_ed_new_string(value_c.as_ptr()) };
                    let s = vm.get_string(handle).unwrap_or(core::ptr::null());
                    // `SV_Precache_Model` can Host_Error (`Mod_ForName`'s
                    // `crash` path when this is the first precache slot), so
                    // it is Host_Guard-wrapped, unlike its two siblings here.
                    let mut precache_idx: c_int = 0;
                    // SAFETY: `s` is either null (matching PR_GetString's
                    // behaviour on an invalid handle) or a NUL-terminated
                    // engine string; `precache_idx` is valid for one write.
                    let guard =
                        unsafe { ed::PREdictDispatch_Glue_PrecacheModel(s, &mut precache_idx) };
                    if guard != 0 {
                        *detail = guard;
                        break 'outer PREDD_ERR_GUARD;
                    }
                } else if loading && keyname.as_slice() == b"_precache_sound" {
                    let value_c = CString::new(value_tok.clone()).unwrap_or_default();
                    // SAFETY: `value_c` is NUL-terminated.
                    let handle = unsafe { quake_rs_ed_new_string(value_c.as_ptr()) };
                    let s = vm.get_string(handle).unwrap_or(core::ptr::null());
                    // SAFETY: `s` is either null or a NUL-terminated engine
                    // string, as above; safe as a leaf call only because
                    // `loading` gates out `SV_Precache_Sound`'s sole
                    // Host_Error-adjacent code (see the module doc comment).
                    unsafe { ed::SV_Precache_Sound(s) };
                }
            }
            continue;
        }

        if keyname.as_slice() == b"alpha" {
            let value_c = CString::new(value_tok.clone()).unwrap_or_default();
            // SAFETY: atof is a leaf libc call.
            let a = unsafe { c::cvar_cmd::atof(value_c.as_ptr()) };
            arena.set_alpha(id, entalpha_encode(a));
        }

        let keyname_c = CString::new(keyname.clone()).unwrap_or_default();
        // SAFETY: precondition.
        let Some(key) = (unsafe { find_field(&keyname_c) }) else {
            // SAFETY: matches C's `qcvm == &sv.qcvm` test.
            let is_server_loading = unsafe {
                c::PRLoad_Glue_IsServerVM(c::qcvm) && ed::PREdictDispatch_Glue_ServerLoading()
            };
            let handled = if keyname.as_slice() == b"traileffect" && is_server_loading {
                // SAFETY: `extfields.traileffectnum` is a word offset from
                // `&ent->v`, or -1 when the progs never bound it
                // (GetEdictFieldValue's fldofs < 0 guard).
                let ofs = unsafe { (*vm.as_ptr()).extfields.traileffectnum };
                if ofs >= 0 {
                    let value_c = CString::new(value_tok.clone()).unwrap_or_default();
                    // SAFETY: leaf call; registers a particle-effect name.
                    let v = unsafe { ed::PF_SV_ForceParticlePrecache(value_c.as_ptr()) } as f32;
                    arena.set_field_f32(id, (ofs as usize) * 4, v);
                }
                true
            } else if keyname.as_slice() == b"emiteffect" && is_server_loading {
                // SAFETY: as above.
                let ofs = unsafe { (*vm.as_ptr()).extfields.emiteffectnum };
                if ofs >= 0 {
                    let value_c = CString::new(value_tok.clone()).unwrap_or_default();
                    // SAFETY: as above -- leaf call, registers a
                    // particle-effect name.
                    let v = unsafe { ed::PF_SV_ForceParticlePrecache(value_c.as_ptr()) } as f32;
                    arena.set_field_f32(id, (ofs as usize) * 4, v);
                }
                true
            } else {
                false
            };
            if !handled
                && !keyname.starts_with(b"sky")
                && keyname.as_slice() != b"fog"
                && keyname.as_slice() != b"alpha"
            {
                pending.push((true, join(&[b"\"", &keyname, b"\" is not a field\n"])));
            }
            continue;
        };

        if anglehack {
            // COMPAT: C's anglehack builds this via `strcpy` into a 32-byte
            // stack buffer (`Quake/pr_edict.c:865-869`) -- an unbounded copy
            // of `com_token` (up to 4095 bytes) into 32 bytes, i.e. a stack
            // buffer overflow. The final `q_snprintf (com_token, 32, "0 %s
            // 0", temp)` is itself bounded, and reads through `temp`'s NUL
            // terminator regardless of where the overflowing `strcpy` placed
            // it, so the *observable* formatted-and-truncated-to-31-bytes
            // output is the same for any input that does not crash the C
            // binary outright. Reproduced as that safe, bounded truncation:
            // `"0 " + value + " 0"`, capped at 31 bytes, with no unbounded
            // intermediate copy.
            let mut buf = Vec::with_capacity(4 + value_tok.len());
            buf.extend_from_slice(b"0 ");
            buf.extend_from_slice(&value_tok);
            buf.extend_from_slice(b" 0");
            buf.truncate(31);
            value_tok = buf;
        }

        let value_c = CString::new(value_tok).unwrap_or_default();
        // SAFETY: `arena`'s base/stride are the live qcvm_t's; `id` is below
        // `count` since the caller only ever passes an edict it owns.
        let dest_base = unsafe {
            arena
                .base()
                .add(id.0 as usize * arena.stride() + EDICT_V_OFFSET) as *mut c_int
        };
        // SAFETY: matches C's `qcvm != &sv.qcvm` test.
        let zoned = unsafe { !c::PRLoad_Glue_IsServerVM(c::qcvm) };
        let mut epair_detail: c_int = 0;
        // SAFETY: `dest_base + key.ofs` stays inside this edict's field
        // block for every def `ED_FindField` hands back; `value_c` is
        // NUL-terminated.
        let epair_status = unsafe {
            quake_rs_ed_parse_epair(
                dest_base,
                key.type_,
                key.ofs,
                key.s_name,
                value_c.as_ptr(),
                zoned,
                &mut epair_detail,
            )
        };
        match epair_status {
            PRPARSE_OK => continue,
            PRPARSE_FALSE => break 'outer PREDD_ERR_EPAIR_PARSE,
            PRPARSE_ERR_ENTITY_RANGE => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_ENTITY_RANGE;
            }
            PRPARSE_ERR_BAD_EDICT_NUM => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_BAD_EDICT_NUM;
            }
            PRPARSE_ERR_FREELIST_FULL => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_FREELIST_FULL;
            }
            PRPARSE_ERR_FREELIST_OVER_MAX => {
                *detail = epair_detail;
                break 'outer PREDD_ERR_FREELIST_OVER_MAX;
            }
            _ => break 'outer PREDD_ERR_EPAIR_PARSE,
        }
    };

    if status == PREDD_OK && !init {
        // SAFETY: `id` was allocated by the caller; the free list lives
        // inside the same ambient qcvm_t, and no other Rust reference to it
        // is live.
        let free_list = unsafe { &mut (*vm.as_ptr()).free_list };
        let time = vm.time();
        alloc::ed_free(free_list, &mut arena, id, time, &mut |e| {
            // SAFETY: `e` is below num_edicts (it is the edict this call
            // just parsed); SV_UnlinkEdict does not raise.
            unsafe { c::PRParse_Glue_UnlinkEdict(e.0 as c_int) }
        });
    }

    // SAFETY: caller's frame owns it.
    unsafe { *out_data = data };
    flush(&mut pending);
    status
}
