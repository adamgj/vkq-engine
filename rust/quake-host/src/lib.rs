//! host loop. Stays a library crate: Rust migration Phase 9 M6 put the
//! process entry (`main`/`WinMain`, `main_sdl.c`'s port) in
//! `quake-platform::main_sdl` and its exported symbols in `quake-capi`, and
//! Meson still links the `quake_rs` staticlib into `vkqr-engine`
//! (`docs/ai/plans/rust-conversion-phase-9.md`, D6).
//!
//! Rust migration Phase 0 stub: populated from Phase 1 onward (ROADMAP.md).
