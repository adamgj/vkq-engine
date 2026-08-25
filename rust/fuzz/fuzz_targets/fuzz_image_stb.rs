//! Image_DecodeSTB pipeline fuzzer (Phase 3 M8, AC11): the pure Rust half
//! of the stb seam — stbi sniffing, the hand-ported TGA decoder, and the
//! PNG structural walk + `png`-crate decode — over arbitrary bytes. The
//! engine builds with panic = "abort", so any panic found here (including
//! one inside the `png` crate under our exact configuration) would abort a
//! running engine on hostile mod content: no-panic is the invariant, plus
//! determinism of the full outcome.
//!
//! JPEG classification is exercised (the sniffer must not panic) but the
//! decode stays on the C stb fallback, which cannot run under libFuzzer
//! (the M7 design note: no C in fuzz targets).

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_image::{png_stb, stb_sniff, tga};

fuzz_target!(|data: &[u8]| {
    let format = stb_sniff::classify(data);
    assert_eq!(format, stb_sniff::classify(data), "classification stable");

    match format {
        stb_sniff::Format::Tga => {
            // dims are u16 fields; cap the legal-allocation blowup
            let dim = |lo: usize| -> u64 {
                u64::from(data.get(lo).copied().unwrap_or(0))
                    | (u64::from(data.get(lo + 1).copied().unwrap_or(0)) << 8)
            };
            if dim(12) * dim(14) * 4 > (64 << 20) {
                return;
            }
            assert_eq!(tga::decode(data), tga::decode(data));
        }
        stb_sniff::Format::Png => {
            // IHDR dims sit at fixed offsets right after the first chunk
            // header when present; stb's own guard allows ~1 GiB pixel
            // buffers, too rich for a fuzz round
            let dim = |be: usize| -> u64 {
                let mut v = 0u64;
                for i in 0..4 {
                    v = (v << 8) | u64::from(data.get(be + i).copied().unwrap_or(0));
                }
                v
            };
            if dim(16).saturating_mul(dim(20)).saturating_mul(8) > (64 << 20) {
                return;
            }
            assert_eq!(png_stb::decode(data), png_stb::decode(data));
        }
        stb_sniff::Format::Jpeg | stb_sniff::Format::Unknown => {}
    }
});
