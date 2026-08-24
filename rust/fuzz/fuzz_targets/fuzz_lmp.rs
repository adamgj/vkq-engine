//! LMP/QPIC decode fuzzer (Phase 3 M7, D11 / AC3): the pure quake-image LMP
//! decision the `Image_DecodeLMP` shim uses. The size-mismatch NULL return
//! and the valid-image accept are the only two outcomes; this asserts they
//! are internally consistent for every fuzzed `(bytes, file_size)`. The
//! C-via-FFI differential lives in `image_differential`/`formats_corpus`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_image::lmp::{self, Lmp};

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    // fuzz the com_filesize the caller truncates to int independently of the
    // real buffer length, exactly like the shim receives it
    let file_size = i32::from_le_bytes(data[0..4].try_into().unwrap());
    let body = &data[4..];

    match lmp::decode(body, file_size) {
        Ok(Lmp::Image {
            width,
            height,
            pixels,
        }) => {
            // C computes pix in wrapping u32; an accepted image reports
            // exactly that many pixels, and the accept condition is
            // file_size == 8 + pix (both promoted to 64-bit unsigned)
            let pix = width.wrapping_mul(height);
            assert_eq!(pixels.len() as u64, u64::from(pix));
            assert_eq!(file_size as i64 as u64, 8 + u64::from(pix));
        }
        Ok(Lmp::SizeMismatch) => {
            // the NULL-return path: header present, size check failed
            assert!(body.len() >= lmp::HEADER_SIZE);
        }
        Err(lmp::Error::NotValid) => {
            // either too short for the header, or the pixel slice the size
            // check blessed still came up short (the C Sys_Error path)
        }
    }
});
