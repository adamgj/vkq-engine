//! ash renderer: gl_heap suballocator, texmgr, pipelines, frame graph
//!
//! Rust migration Phase 8 (ROADMAP.md, ADR-015): populated slice by slice
//! behind `-Duse_rust_render`. M3 lands [`heap`], the device-memory
//! suballocator; the C ABI shims live in `quake-capi`.

pub mod heap;
