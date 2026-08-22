//! #[repr(C)] wire/disk/ABI mirrors + const layout tests
//!
//! Rust migration Phase 1 (ROADMAP.md): hand-written mirrors of compat-critical
//! C structs, each with const size/offset assertions (ADR-011).

#![forbid(unsafe_code)] // ADR-004: pure crate

pub mod bspfile;
pub mod fs;
pub mod json;
pub mod model_mem;
pub mod modelgen;
pub mod plane;
pub mod spritegn;
pub mod wad;

pub use plane::MPlane;
