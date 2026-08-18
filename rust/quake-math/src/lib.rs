//! mathlib: vec3 ops, AngleVectors, BoxOnPlaneSide (bit-exact)
//!
//! Rust migration Phase 1 (ROADMAP.md): port of `Quake/mathlib.c` plus the
//! anorms.h table. FFI shims live in quake-capi; libm goes through
//! quake-c-sys (ADR-010).

#![forbid(unsafe_code)] // ADR-004: pure crate

pub mod anorms;
pub mod mathlib;

pub use mathlib::*;
