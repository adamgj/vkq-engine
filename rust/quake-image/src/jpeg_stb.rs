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
    /// stb: "outofmem" (load_jpeg_image's stbi__malloc_mad3(4, x, y, 1)
    /// output buffer overflows an int)
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

/// stb's `stbi__mad3sizes_valid`: `a*b*c + add` fits a non-negative int.
fn mad3_valid(a: i64, b: i64, c: i64, add: i64) -> bool {
    mul2_valid(a, b) && mul2_valid(a * b, c) && a * b * c <= i64::from(i32::MAX) - add
}

/// stb's `stbi__get_marker` consumes any number of 0xFF fill bytes before
/// the SOI (the sniffer accepted exactly that shape); zune requires the
/// stream to start FF D8, so drop all but the last fill byte. Expressed as
/// a property of the leading 0xFF run rather than "the first 0xD8 in the
/// file", so it stays correct if the sniffer is ever loosened.
fn strip_fill_bytes(file: &[u8]) -> &[u8] {
    let fill = file.iter().take_while(|&&b| b == 0xFF).count();
    &file[fill.saturating_sub(1)..]
}

/// Decode the whole resource, already classified as JPEG by
/// [`crate::stb_sniff`].
pub fn decode(file: &[u8]) -> Result<Jpeg, Error> {
    let file = strip_fill_bytes(file);

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
    // With req_comp = 4, load_jpeg_image takes n = req_comp and allocates
    // stbi__malloc_mad3(n, x, y, 1) = 4*x*y + 1 directly (no x*y*3
    // intermediate); a NULL from its mad3 overflow check is "outofmem".
    // Reachable: 4*65535*65535 overflows an int.
    if !mad3_valid(4, w as i64, h as i64, 1) {
        return Err(Error::OutOfMem);
    }

    let rgba = decoder
        .decode()
        .map_err(|e| Error::Crate(format!("{e:?}")))?;
    // The shim publishes (w, h) and hands the caller a Mem_Alloc buffer of
    // rgba.len(); every consumer then reads w*h*4. A short buffer from the
    // crate would be an out-of-bounds read, so reject rather than assert —
    // debug_assert would be compiled out in the shipping profile.
    if rgba.len() != w.saturating_mul(h).saturating_mul(4) {
        return Err(Error::Crate("unexpected RGBA output size".into()));
    }

    Ok(Jpeg {
        width: info.width as i32,
        height: info.height as i32,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mad3_matches_stbs_output_buffer_bound() {
        // stb: stbi__malloc_mad3(4, x, y, 1) -> 4*x*y + 1 must fit an int
        let max = i64::from(i32::MAX);
        assert!(mad3_valid(4, 1, 1, 1));
        // the largest JPEG dimensions overflow, as they do in stb
        assert!(!mad3_valid(4, 65_535, 65_535, 1));
        // exactly at the boundary: 4*x*y + 1 == INT_MAX is still valid
        let (x, y) = ((max - 1) / 4, 1);
        assert!(mad3_valid(4, x, y, 1));
        assert!(!mad3_valid(4, x + 1, y, 1));
    }

    #[test]
    fn leading_fill_bytes_are_stripped_to_a_single_soi_marker() {
        // stb's get_marker eats any run of 0xFF before the SOI; the wrapper
        // keeps exactly one so zune sees FF D8
        for fill in 1..6usize {
            let mut v = vec![0xFFu8; fill];
            v.extend_from_slice(&[0xD8, 0xFF, 0xD9]);
            assert_eq!(strip_fill_bytes(&v), &[0xFF, 0xD8, 0xFF, 0xD9]);
        }
        // a 0xD8 inside the body is not mistaken for the SOI
        assert_eq!(
            strip_fill_bytes(&[0xFF, 0xD8, 0x11, 0xD8]),
            &[0xFF, 0xD8, 0x11, 0xD8]
        );
        // degenerate inputs the sniffer never produces must not panic
        assert_eq!(strip_fill_bytes(&[]), &[] as &[u8]);
        assert_eq!(strip_fill_bytes(&[0xFF, 0xFF]), &[0xFF]);
        assert_eq!(strip_fill_bytes(&[0x00, 0xD8]), &[0x00, 0xD8]);
    }
}
