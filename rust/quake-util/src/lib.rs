//! crc16, folded MD4, strl*, q_ctype, C-printf-compatible float formatter (ADR-005)
//!
//! Rust migration Phase 1 (ROADMAP.md): pure leaf utilities ported from C.
//! FFI shims for these live in quake-capi, never here.

#![forbid(unsafe_code)] // ADR-004: pure crate

pub mod crc;
pub mod hash_map;
pub mod json;
pub mod mdfour;
pub mod printf;
pub mod qctype;
pub mod strl;
