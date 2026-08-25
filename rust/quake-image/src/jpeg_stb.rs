//! JPEG decode behind the stb seam (Phase 3 M8, ADR-012/D8) via `zune-jpeg`.
//!
//! COMPAT (owner decision, 2026-08-24, task-plan amendment log): unlike TGA
//! and PNG this leg is NOT bit-exact vs stb — stb's fixed-point IDCT,
//! upsampler and YCbCr rounding differ from zune-jpeg's libjpeg-turbo-style
//! pipeline. The relaxed gate is: accept/reject parity, identical
//! dimensions, and a bounded per-channel pixel delta, pinned by the
//! `image_crate_differential` JPEG cases. Failure-reason text is
//! crate-originated and masked in the differential.
//!
//! The wrapper ports stb's two pre-decode guards so the *decision* stays
//! stb's where it is cheap to guarantee: the STBI_MAX_DIMENSIONS check
//! ("too large") and the output-buffer int-overflow check ("outofmem").
//! zune's own width/height limits are raised to the JPEG format maximum so
//! they can never reject an image stb would accept; strict mode is off
//! (stb is lenient about e.g. data past EOI).

use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

#[derive(Debug, PartialEq, Eq)]
pub struct Jpeg {
    pub width: i32,
    pub height: i32,
    /// width * height * 4 RGBA bytes (stb req_comp = 4)
    pub rgba: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// stb: "too large" (STBI_MAX_DIMENSIONS — unreachable for JPEG's
    /// 16-bit dimension fields, ported for fidelity)
    TooLarge,
    /// stb: "outofmem" (stbi__mad3sizes_valid overflow on the decode or
    /// RGBA conversion buffer)
    OutOfMem,
    /// zune-jpeg reject: same class as an stb decode failure; the reason
    /// text is the crate's own (masked in the differential)
    Crate(String),
}

const STBI_MAX_DIMENSIONS: usize = 1 << 24;

fn mul2_valid(a: i64, b: i64) -> bool {
    if b == 0 {
        return true;
    }
    a <= i64::from(i32::MAX) / b
}

/// Decode the whole resource, already classified as JPEG by
/// [`crate::stb_sniff`].
pub fn decode(file: &[u8]) -> Result<Jpeg, Error> {
    // stb's get_marker consumes any number of 0xFF fill bytes before the
    // SOI (the sniffer accepted exactly that shape); zune requires the
    // stream to start FF D8, so strip the extra fill bytes
    let soi = file.iter().position(|&b| b == 0xD8).unwrap_or(1);
    let file = &file[soi.saturating_sub(1)..];

    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(65_535)
        .set_max_height(65_535)
        .set_strict_mode(false);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(file), options);
    decoder
        .decode_headers()
        .map_err(|e| Error::Crate(format!("{e:?}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| Error::Crate("no info".into()))?;
    let (w, h) = (info.width as usize, info.height as usize);

    // stb's stbi__process_frame_header guards, in its order
    if h > STBI_MAX_DIMENSIONS || w > STBI_MAX_DIMENSIONS {
        return Err(Error::TooLarge);
    }
    // stb allocates x*y*n (n = 3 for color, 1 for gray) and then converts
    // to x*y*4; both products must fit an int
    let n = if info.components >= 3 { 3 } else { 1 };
    if !mul2_valid(w as i64 * h as i64, n) || !mul2_valid(w as i64 * h as i64, 4) {
        return Err(Error::OutOfMem);
    }

    let rgba = decoder
        .decode()
        .map_err(|e| Error::Crate(format!("{e:?}")))?;
    debug_assert_eq!(rgba.len(), w * h * 4, "RGBA output requested");

    Ok(Jpeg {
        width: info.width as i32,
        height: info.height as i32,
        rgba,
    })
}
