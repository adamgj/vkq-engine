//! cbindgen-exported extern "C" shims; builds the libquake_rs staticlib.
//!
//! ADR-011: every `#[no_mangle] extern "C"` export of the workspace lives in
//! this crate, and each shim replicates its C original's signature exactly so
//! call sites keep compiling against the existing engine headers (crc.h,
//! strl_fn.h, ...). `scripts/harness/check_capi_signatures.sh` enforces that
//! by compiling the generated quake_rs.h against those headers in one TU.

// DO_USERDIRS adds per-user lookup steps to LOC_LoadFile that the Rust fs
// does not implement yet; fail loudly rather than silently diverge
#[cfg(all(feature = "fs", feature = "userdirs"))]
compile_error!(
    "the Rust filesystem does not implement DO_USERDIRS yet; build with -Duse_rust_fs=disabled"
);

#[cfg(feature = "engine-alloc")]
pub mod alloc;
pub mod cfgfile;
pub mod crc;
#[cfg(feature = "fs")]
pub mod fs;
#[cfg(feature = "fs")]
pub mod fs_stdio;
pub mod hash_map;
pub mod json;
#[cfg(feature = "fs")]
pub mod loc;
pub mod mathlib;
pub mod mdfour;
#[cfg(feature = "fs")]
pub mod steam;
pub mod strl;
pub mod wad;

/// Phase 0 link probe: proves the staticlib is linked and its symbols
/// resolve from C. Returns the quake-capi crate ABI version.
#[no_mangle]
pub extern "C" fn QuakeRS_Version() -> u32 {
    0
}
