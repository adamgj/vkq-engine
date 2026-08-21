//! image decode/encode orchestration (ADR-012)
//!
//! Rust migration Phase 3: pure PCX/LMP decoders ported from
//! Quake/image_decode.c (M2). PNG/TGA/JPG decode stays in C (image_stb.c)
//! until M8.

#![forbid(unsafe_code)] // ADR-004: pure crate

pub mod lmp;
pub mod pcx;
