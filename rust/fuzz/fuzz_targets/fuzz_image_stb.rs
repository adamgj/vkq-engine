//! Image_DecodeSTB pipeline fuzzer (Phase 3 M8, AC11): the pure Rust half
//! of the stb seam — stbi sniffing, the hand-ported TGA decoder, the PNG
//! structural walk + `png`-crate decode, and the zune-jpeg wrapper — over
//! arbitrary bytes. The engine builds with panic = "abort", so any panic
//! found here (including one inside the `png` or `zune-jpeg` crates under
//! our exact configuration) would abort a running engine on hostile mod
//! content: no-panic is the invariant, plus determinism of the full
//! outcome.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_image::{jpeg_stb, png_stb, stb_sniff, tga};

/// Cheap SOF dimension scan so a tiny fuzz input declaring 65535x65535
/// cannot ask for a multi-GiB legal allocation: walk the marker stream to
/// the first SOF and read its dims.
fn jpeg_dims(data: &[u8]) -> Option<(u64, u64)> {
    let mut i = 2usize;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = data[i + 1];
        match m {
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                let h = u64::from(data[i + 5]) << 8 | u64::from(data[i + 6]);
                let w = u64::from(data[i + 7]) << 8 | u64::from(data[i + 8]);
                return Some((w, h));
            }
            0xDA | 0xD9 => return None,
            0xD0..=0xD8 | 0x01 | 0xFF => i += 2,
            _ => {
                let len = usize::from(data[i + 2]) << 8 | usize::from(data[i + 3]);
                i += 2 + len.max(2);
            }
        }
    }
    None
}

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
            // walk the chunk stream to the IHDR (it need not be the first
            // chunk: CgBI or a large ancillary chunk can precede it) so the
            // guard cannot be bypassed by reordering; the decoder's own
            // "outofmem" gate bounds outputs to i32::MAX, still too rich
            // for a fuzz round under the rss limit
            let be32 = |i: usize| -> u64 {
                let mut v = 0u64;
                for k in 0..4 {
                    v = (v << 8) | u64::from(data.get(i + k).copied().unwrap_or(0));
                }
                v
            };
            let mut pos = 8usize;
            let mut dims = None;
            for _ in 0..64 {
                let (len, ty) = (be32(pos), be32(pos + 4));
                if ty == 0x4948_4452 {
                    dims = Some((be32(pos + 8), be32(pos + 12)));
                    break;
                }
                pos = pos.saturating_add(12).saturating_add(len as usize);
                if pos >= data.len() {
                    break;
                }
            }
            // dims stays None when no IHDR turns up in the first 64 chunks
            // (or the walk runs off the end): that input is not size-gated
            // here, but it cannot reach a decode allocation either — every
            // other chunk arm rejects with "first not IHDR" while `first`
            // holds, so an IHDR-less stream dies at chunk 1
            if let Some((w, h)) = dims {
                if w.saturating_mul(h).saturating_mul(8) > (64 << 20) {
                    return;
                }
            }
            assert_eq!(png_stb::decode(data), png_stb::decode(data));
        }
        stb_sniff::Format::Jpeg => {
            if let Some((w, h)) = jpeg_dims(data) {
                if w.saturating_mul(h).saturating_mul(4) > (64 << 20) {
                    return;
                }
            }
            assert_eq!(jpeg_stb::decode(data), jpeg_stb::decode(data));
        }
        stb_sniff::Format::Unknown => {}
    }
});
