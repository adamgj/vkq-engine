//! Sound engine (Rust migration Phase 4, ROADMAP.md).
//!
//! Pure ports of the C sound code: WAV parsing (`snd_mem.c`'s GetWavinfo),
//! the sfx resampler, the software mixer, channel/spatialization logic, the
//! codec framework (ADR-014), and background music. The FFI shims that bolt
//! these onto the engine live in `quake-capi` behind its `snd` feature.
//!
//! Everything here is bit-compatibility-critical (ADR-010): mixer arithmetic
//! reproduces the C's truncating divisions and float evaluation order
//! exactly, and transcendentals go through `quake_c_sys::libm` (the platform
//! libm the C build links), never a Rust reimplementation.

#![forbid(unsafe_code)] // ADR-004: pure crate; unsafe lives in quake-capi

pub mod mix;
pub mod resample;
pub mod wav;
