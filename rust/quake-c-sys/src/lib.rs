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

#[allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
mod generated;

pub use generated::*;
