//! Platform libm / CRT call-throughs.
//!
//! // COMPAT: ADR-010 — transcendentals whose results are simulation-visible
//! must call the platform libm (what the C build linked), never Rust's
//! `f64::sin` etc., so Rust output stays bit-identical to the C build on the
//! same platform. `strtod` is here for the same reason: `json.c` parses
//! numbers with the platform `strtod` and acceptance/rounding must not change
//! (ADR-012).
//!
//! These are C-standard-library declarations, not engine symbols, so they are
//! hand-written rather than bindgen-generated (ADR-011 amendment, Phase 1).
//! The safe wrappers exist so `forbid(unsafe_code)` crates (quake-math,
//! quake-util) can call them.

use core::ffi::c_char;

mod ffi {
    use core::ffi::{c_char, c_double};
    extern "C" {
        pub fn sin(x: c_double) -> c_double;
        pub fn cos(x: c_double) -> c_double;
        pub fn atan2(y: c_double, x: c_double) -> c_double;
        pub fn sqrt(x: c_double) -> c_double;
        pub fn acos(x: c_double) -> c_double;
        pub fn floor(x: c_double) -> c_double;
        pub fn fabs(x: c_double) -> c_double;
        pub fn exp(x: c_double) -> c_double;
        pub fn log(x: c_double) -> c_double;
        pub fn sqrtf(x: f32) -> f32;
        pub fn sinf(x: f32) -> f32;
        pub fn cosf(x: f32) -> f32;
        pub fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> c_double;
    }
}

pub fn sin(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::sin(x) }
}

pub fn cos(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::cos(x) }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::atan2(y, x) }
}

pub fn sqrt(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::sqrt(x) }
}

pub fn acos(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::acos(x) }
}

pub fn sqrtf(x: f32) -> f32 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::sqrtf(x) }
}

pub fn floor(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::floor(x) }
}

pub fn exp(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::exp(x) }
}

pub fn log(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::log(x) }
}

pub fn fabs(x: f64) -> f64 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::fabs(x) }
}

pub fn sinf(x: f32) -> f32 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::sinf(x) }
}

pub fn cosf(x: f32) -> f32 {
    // SAFETY: pure libm function, no preconditions
    unsafe { ffi::cosf(x) }
}

/// Platform `strtod (nptr, NULL)` over a NUL-terminated byte buffer.
/// Only the no-endptr form json.c uses is exposed.
pub fn strtod(nul_terminated: &[u8]) -> f64 {
    assert_eq!(
        nul_terminated.last(),
        Some(&0),
        "buffer must be NUL-terminated"
    );
    // SAFETY: the assert guarantees a NUL within the readable range, and
    // endptr = NULL is allowed by the C standard
    unsafe {
        ffi::strtod(
            nul_terminated.as_ptr() as *const c_char,
            core::ptr::null_mut(),
        )
    }
}
