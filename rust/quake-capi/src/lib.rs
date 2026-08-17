//! cbindgen-exported extern "C" shims; builds the libquake_rs staticlib
//!
//! Rust migration Phase 0 stub: populated from Phase 1 onward (ROADMAP.md).

/// Phase 0 link probe: proves the staticlib is linked and its symbols
/// resolve from C. Returns the quake-capi crate ABI version.
#[no_mangle]
pub extern "C" fn QuakeRS_Version() -> u32 {
    0
}
