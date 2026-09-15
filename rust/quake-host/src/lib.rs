//! host loop. Stays a library crate: Rust migration Phase 9 M6 put the
//! process entry (`main`/`WinMain`, `main_sdl.c`'s port) in
//! `quake-platform::main_sdl` and its exported symbols in `quake-capi`, and
//! Meson still links the `quake_rs` staticlib into `vkqr-engine`
//! (`docs/ai/plans/rust-conversion-phase-9.md`, D6).
//!
//! Phase 9 M7 (ADR-009): [`error::HostError`], the typed outcome of a
//! `Host_Guard`ed call, which that loop consumes in place of the raw
//! `int` status. The loop itself is still `quake-platform::main_sdl`;
//! `docs/rust-migration/setjmp-inventory.md` records what of the C
//! `setjmp`/`longjmp` machinery remains and why.

pub mod error;
