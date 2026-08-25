//! image decode/encode orchestration (ADR-012)
//!
//! Rust migration Phase 3: pure PCX/LMP decoders ported from
//! Quake/image_decode.c (M2); the stb_image sniff/dispatch chain and the
//! per-format PNG/TGA/JPG decoders behind it (M8). PNG *encode* stays on
//! lodepng in C (ADR-012).

#![forbid(unsafe_code)] // ADR-004: pure crate

pub mod lmp;
pub mod pcx;
pub mod stb_sniff;
