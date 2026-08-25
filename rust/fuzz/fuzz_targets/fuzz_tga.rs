//! TGA decode fuzzer (Phase 3 M8, AC11): drives the pure quake-image stb
//! probe chain and the hand-ported TGA decoder the `Image_DecodeSTB` shim
//! dispatches on. The true C-via-FFI differential lives in `quake-ctest`'s
//! `image_crate_differential` and the `formats_corpus` gate; this target is
//! coverage-guided exploration of the classify/decode decisions, which must
//! never panic, keep every write in bounds, and be deterministic.
//!
//! The decoder runs only when the ported stbi__tga_test accepts — the shim's
//! own dispatch contract. The size guard mirrors the fuzz_pcx precedent: the
//! stb mad3 check already bounds accepted images to i32::MAX bytes, but a
//! 65535x65535x1 grey image is ~4 GiB of legal allocation, so oversized
//! accepts are checked for the decision only, not decoded.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_image::{stb_sniff, tga};

fuzz_target!(|data: &[u8]| {
    // classify must never panic, on any input
    let format = stb_sniff::classify(data);
    if format != stb_sniff::Format::Tga {
        return;
    }

    // keep the fuzz allocation sane: dims are u16 fields at fixed offsets
    let dim = |lo: usize| -> u64 {
        u64::from(data.get(lo).copied().unwrap_or(0))
            | (u64::from(data.get(lo + 1).copied().unwrap_or(0)) << 8)
    };
    if dim(12) * dim(14) * 4 > (64 << 20) {
        return;
    }

    // Must return Ok or a reject reason — never panic (a panic would abort
    // the engine: panic = "abort") — and must be deterministic.
    let first = tga::decode(data);
    let second = tga::decode(data);
    assert_eq!(first, second);
    if let Ok(t) = first {
        assert!(t.width >= 1 && t.height >= 1, "tga_test guarantees dims");
        assert_eq!(
            t.rgba.len(),
            t.width as usize * t.height as usize * 4,
            "output is exactly w*h*4 RGBA"
        );
    }
});
