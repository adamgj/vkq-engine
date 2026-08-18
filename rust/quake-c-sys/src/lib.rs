//! bindgen externs to remaining C (shrinks to empty)
//!
//! `generated` is the committed output of `scripts/gen_c_bindings.sh`
//! (ADR-011: only this crate declares engine C symbols; CI regenerates and
//! diffs it). `libm` holds hand-written platform libm/CRT declarations with
//! safe wrappers for the `forbid(unsafe_code)` crates (ADR-010).

pub mod libm;

/// Engine globals whose C types cannot be represented portably in the
/// committed bindings (platform-dependent array lengths); only the base
/// address is used from Rust.
pub mod manual {
    use core::ffi::c_char;
    extern "C" {
        /// char com_basedir[MAX_OSPATH] (MAX_OSPATH is PATH_MAX)
        pub static mut com_basedir: [c_char; 0];
    }
}

/// The engine's vendored mimalloc (amalgamated into mem.c's translation unit
/// via `#include "mimalloc/static.c"`; the `mi_*` symbols have external
/// linkage there). Only linked in the Meson mixed build — see quake-capi's
/// `engine-alloc` feature.
pub mod mi {
    use core::ffi::c_void;
    extern "C" {
        /// C: `void *mi_malloc_aligned (size_t size, size_t alignment)`
        pub fn mi_malloc_aligned(size: usize, alignment: usize) -> *mut c_void;
        /// C: `void *mi_zalloc_aligned (size_t size, size_t alignment)`
        pub fn mi_zalloc_aligned(size: usize, alignment: usize) -> *mut c_void;
        /// C: `void *mi_realloc_aligned (void *p, size_t newsize, size_t alignment)`
        pub fn mi_realloc_aligned(p: *mut c_void, newsize: usize, alignment: usize) -> *mut c_void;
        /// C: `void mi_free (void *p)`
        pub fn mi_free(p: *mut c_void);
    }
}

#[allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
mod generated;

pub use generated::*;
