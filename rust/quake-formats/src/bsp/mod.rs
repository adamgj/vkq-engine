//! Brush/BSP lump parsing, ported from the brush range of
//! `Quake/model_parse.c` (Rust migration Phase 3 M3).
//!
//! Pure layer: every function here parses a lump byte slice (or transforms
//! already-parsed values) into owned records; no allocation into engine
//! memory, no I/O, no globals. The `quake-capi` shim replays the records
//! into `Mem_Alloc`-backed `quake_types::model_mem` structs with the exact
//! write order and partial-failure state of the C loaders.
//!
//! The C loaders trust lump directories and read through raw pointers;
//! malformed inputs reach undefined behavior there (out-of-bounds reads).
//! This port bounds every access to the given slice — divergence is
//! confined to inputs where the C behavior is UB, matching the policy set
//! for the image decoders (task plan amendment log).

pub mod extents;
pub mod lighting;
pub mod lumps;
pub mod textures;
pub mod vis;

/// The C `bsp2` int: 0 (BSP29/BSP30/Q64), 1 (2PSB), 2 (BSP2)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bsp2 {
    No,
    L1,
    L2,
}

/// C: `Sys_Error`/`Host_Error` "funny lump size in %s" — the shim picks the
/// message prefix and the fatal flavor per call site
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunnySize;

pub(crate) fn i16_at(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

pub(crate) fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
