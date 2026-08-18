//! C ABI shims for `Quake/strlcpy.c` / `Quake/strlcat.c` (declared in
//! `Quake/strl_fn.h`, included by common.h — the single most-called Phase 1
//! API; OpenBSD return semantics are relied on by callers).

use core::ffi::{c_char, CStr};

/// C: `size_t q_strlcpy (char *dst, const char *src, size_t siz);`
///
/// # Safety
/// `src` must be a NUL-terminated C string; `dst` must be valid for `siz`
/// bytes (or unused when `siz == 0`).
#[no_mangle]
pub unsafe extern "C" fn q_strlcpy(dst: *mut c_char, src: *const c_char, siz: usize) -> usize {
    // SAFETY: src is a NUL-terminated C string per the strl_fn.h contract
    let src = unsafe { CStr::from_ptr(src) }.to_bytes();
    let dst = if siz == 0 {
        &mut [][..]
    } else {
        // SAFETY: dst is valid for siz writable bytes per the strl_fn.h contract
        unsafe { core::slice::from_raw_parts_mut(dst as *mut u8, siz) }
    };
    quake_util::strl::strlcpy(dst, src)
}

/// C: `size_t q_strlcat (char *dst, const char *src, size_t siz);`
///
/// # Safety
/// `src` must be a NUL-terminated C string; `dst` must be valid for `siz`
/// bytes (the existing-content scan is bounded by `siz`, like the C).
#[no_mangle]
pub unsafe extern "C" fn q_strlcat(dst: *mut c_char, src: *const c_char, siz: usize) -> usize {
    // SAFETY: src is a NUL-terminated C string per the strl_fn.h contract
    let src = unsafe { CStr::from_ptr(src) }.to_bytes();
    let dst = if siz == 0 {
        &mut [][..]
    } else {
        // SAFETY: dst is valid for siz bytes per the strl_fn.h contract
        unsafe { core::slice::from_raw_parts_mut(dst as *mut u8, siz) }
    };
    quake_util::strl::strlcat(dst, src)
}
