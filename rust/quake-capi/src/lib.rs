//! cbindgen-exported extern "C" shims; builds the libquake_rs staticlib.
//!
//! ADR-011: every `#[no_mangle] extern "C"` export of the workspace lives in
//! this crate, and each shim replicates its C original's signature exactly so
//! call sites keep compiling against the existing engine headers (crc.h,
//! strl_fn.h, ...). `scripts/harness/check_capi_signatures.sh` enforces that
//! by compiling the generated quake_rs.h against those headers in one TU.

pub mod crc;
pub mod hash_map;
pub mod json;
pub mod mathlib;
pub mod mdfour;
pub mod strl;
pub mod wad;

/// Phase 0 link probe: proves the staticlib is linked and its symbols
/// resolve from C. Returns the quake-capi crate ABI version.
#[no_mangle]
pub extern "C" fn QuakeRS_Version() -> u32 {
    0
}
