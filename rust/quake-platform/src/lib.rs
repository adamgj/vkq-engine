//! SDL2+SDL3 platform glue, input, video, sys/pl layers (ADR-017).
//!
//! Phase 4 M9 populates the SDL3 audio backend; the rest of the platform
//! layer arrives in Phase 9. The `sdl2` fallback backend stays C until a
//! use_rust+SDL2 CI leg exists to verify its Rust port.

#[cfg(feature = "sdl3")]
pub mod snd_sdl3;
