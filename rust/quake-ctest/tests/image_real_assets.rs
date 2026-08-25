//! Env-gated real-asset legs of the M8 stb seam (AC11): mounts the
//! re-release depot like md5_differential's real corpus case and runs the
//! two real JPEGs (`vault/dawn.jpg`, `vault/dawn_thumb.jpg` — the only
//! `.jpg` files in the whole depot) through both sides. PNG/TGA real assets
//! are covered bit-for-bit by the formats_corpus gate; JPEG is compared
//! under the owner-relaxed gate here (accept + dims parity + bounded
//! per-channel delta) because stb's and zune-jpeg's IDCT/upsampler rounding
//! differ by design.
//!
//! Skipped silently without QUAKE_GAME_DATA (ADR-019: no game data is
//! committed or copied; only compared in place).

use quake_ctest::drivers as drv;
use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;

fn stb_buf_len(w: i32, h: i32) -> usize {
    ((w as u32).wrapping_mul(h as u32) as usize).wrapping_mul(4)
}

/// The pinned real-JPEG divergence bound: measured 3 (dawn.jpg, 0.63% of
/// bytes differing) and 4 (dawn_thumb.jpg, 5.7%) on darwin-arm64
/// 2026-08-24; headroom for the per-platform SIMD paths of both decoders.
/// A regression that widens it past this fails the gate.
const REAL_JPEG_MAX_DELTA: u8 = 8;

#[test]
fn real_rerelease_jpg_corpus_delta_bounded() {
    let Ok(depot) = std::env::var("QUAKE_GAME_DATA") else {
        eprintln!("QUAKE_GAME_DATA not set; skipping the real-JPEG corpus leg");
        return;
    };
    let root = std::path::PathBuf::from(depot);
    if !root.join("rerelease/id1/pak0.pak").exists() {
        eprintln!("no rerelease/id1 in the depot; skipping");
        return;
    }
    let _guard = ctfs::lock();
    for side in ctfs::BOTH {
        // depot root as base dir, "rerelease/id1" as the gamedir: a literal
        // "id1" would be filtered by COM_ResetGameDirectories (see the M5
        // real-MD5 case)
        ctfs::setup(side, &[&root], 0, c"rerelease/id1");
    }

    for name in [c"vault/dawn.jpg", c"vault/dawn_thumb.jpg"] {
        // SAFETY: pak mounted above; the STB seam soft-fails on both sides
        let c =
            unsafe { drv::image_decode_side(Side::C, name, drv::ImageFormat::Stb, stb_buf_len) };
        // SAFETY: as above
        let r =
            unsafe { drv::image_decode_side(Side::Rust, name, drv::ImageFormat::Stb, stb_buf_len) };
        assert_eq!(
            (
                c.width,
                c.height,
                c.file_size,
                &c.con_log,
                c.error.is_some()
            ),
            (
                r.width,
                r.height,
                r.file_size,
                &r.con_log,
                r.error.is_some()
            ),
            "{name:?}: non-pixel outcome parity"
        );
        assert_eq!(c.open_handles, 0, "{name:?}: handle balance");
        let (cd, rd) = (
            c.data.as_ref().expect("real JPEG decodes in C"),
            r.data.as_ref().expect("real JPEG decodes in Rust"),
        );
        assert_eq!(cd.len(), rd.len(), "{name:?}: buffer size");
        let mut max_delta = 0u8;
        let mut diffs = 0u64;
        for (a, b) in cd.iter().zip(rd.iter()) {
            let d = a.abs_diff(*b);
            max_delta = max_delta.max(d);
            diffs += u64::from(d != 0);
        }
        eprintln!(
            "{name:?}: {}x{} max_delta={max_delta} differing_bytes={diffs}/{}",
            c.width,
            c.height,
            cd.len()
        );
        assert!(
            max_delta <= REAL_JPEG_MAX_DELTA,
            "{name:?}: max per-channel delta {max_delta} exceeds the pinned {REAL_JPEG_MAX_DELTA}"
        );
    }
}
