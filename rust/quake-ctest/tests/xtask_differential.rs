//! Differential tests for the xtask embedded-pak pipeline (Phase 8 M11, task
//! plan D8): `xtask::bintoc::deflate_raw` must produce exactly the bytes
//! `bintoc -c` gets from the vendored miniz (`tdefl_compress_mem_to_heap`
//! with `TDEFL_MAX_PROBES_MASK`), which common_fs.c compiles into the c_ref
//! oracle, on the real `Misc/vq_pak` contents as well as synthetic inputs.
//! The .spv/.c/vkquake.pak identity against Meson's outputs is the
//! scripts/harness/xtask_diff.py gate (AC10).

use core::ffi::{c_int, c_void};

// links the c_ref build (common_fs.c's miniz TU) and its stubs into this binary
extern crate quake_ctest;

extern "C" {
    fn tdefl_compress_mem_to_heap(
        src: *const c_void,
        src_len: usize,
        out_len: *mut usize,
        flags: c_int,
    ) -> *mut c_void;
    fn mz_free(p: *mut c_void);
}

/// `Shaders/bintoc.c`: `tdefl_compress_mem_to_heap (..., TDEFL_MAX_PROBES_MASK)`.
const TDEFL_MAX_PROBES_MASK: c_int = 0xFFF;

fn c_deflate(input: &[u8]) -> Vec<u8> {
    let mut len = 0usize;
    // SAFETY: miniz reads `input.len()` bytes from `input` and returns a
    // heap block of `len` bytes (or null on failure) that mz_free releases.
    unsafe {
        let p = tdefl_compress_mem_to_heap(
            input.as_ptr().cast(),
            input.len(),
            &mut len,
            TDEFL_MAX_PROBES_MASK,
        );
        assert!(!p.is_null(), "tdefl_compress_mem_to_heap failed");
        let out = std::slice::from_raw_parts(p.cast::<u8>(), len).to_vec();
        mz_free(p);
        out
    }
}

fn check_parity(label: &str, input: &[u8]) -> Result<(), String> {
    let c = c_deflate(input);
    let rs = xtask::bintoc::deflate_raw(input);
    let back = miniz_oxide::inflate::decompress_to_vec(&rs).expect("inflate");
    assert!(back == input, "{label}: round trip");
    if c == rs {
        Ok(())
    } else {
        Err(format!(
            "{label}: deflate differs ({} C bytes vs {} Rust bytes)",
            c.len(),
            rs.len()
        ))
    }
}

fn assert_parity(label: &str, input: &[u8]) {
    check_parity(label, input).unwrap_or_else(|e| panic!("{e}"));
}

fn lcg_bytes(seed: u64, len: usize, modulus: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) % modulus) as u8
        })
        .collect()
}

#[test]
fn deflate_matches_miniz_on_synthetic_inputs() {
    assert_parity("empty", b"");
    assert_parity("short text", b"Deflate late");
    assert_parity("zeros", &[0u8; 100_000]);
    assert_parity("runs", &b"abcabcabcabd".repeat(4000));
    assert_parity("noise", &lcg_bytes(1, 70_000, 256));
    assert_parity("low entropy", &lcg_bytes(2, 200_000, 7));
    let mut mixed = lcg_bytes(3, 30_000, 256);
    mixed.extend_from_slice(&[0u8; 30_000]);
    mixed.extend_from_slice(&b"PACK".repeat(10_000));
    mixed.extend_from_slice(&lcg_bytes(4, 30_000, 16));
    assert_parity("mixed", &mixed);
}

/// The one known divergence: miniz_oxide's `flush_block` only falls back to a
/// stored block when the block holds more than 32 bytes, where miniz does so
/// whenever the coded block would not be smaller. An incompressible block of
/// at most 32 bytes therefore codes as a (larger) static block in Rust and a
/// stored block in C; both inflate to the same bytes. This pins the boundary
/// so a miniz_oxide upgrade that closes (or widens) the gap is noticed, and
/// the real-pak test above is what proves `Misc/vq_pak` is not affected.
#[test]
fn deflate_diverges_from_miniz_only_on_tiny_incompressible_blocks() {
    for n in 1..=48 {
        let input = lcg_bytes(n as u64, n, 256);
        let parity = check_parity(&format!("{n} noise bytes"), &input);
        if n <= 32 {
            assert!(
                parity.is_err(),
                "{n} noise bytes unexpectedly matched miniz"
            );
            let c = c_deflate(&input);
            assert_eq!(
                c.len(),
                n + 5,
                "{n} noise bytes: miniz emits a stored block"
            );
        } else {
            parity.unwrap_or_else(|e| panic!("{e}"));
        }
    }
}

#[test]
fn deflate_matches_miniz_on_the_real_pak() {
    let root = xtask::repo_root().join("Misc").join("vq_pak");
    let dir = std::env::temp_dir().join(format!("vkq-xtask-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pak = dir.join("vkquake.pak");
    xtask::pak::build(&pak, &root, &root.join("vq_pak_contents.txt"), None).unwrap();
    let bytes = std::fs::read(&pak).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(&bytes[..4], b"PACK");
    let dirofs = i32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let dirlen = i32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let toc = std::fs::read(root.join("vq_pak_contents.txt")).unwrap();
    assert_eq!(dirofs, 12);
    assert_eq!(dirlen, xtask::pak::entries(&toc).len() * 64);
    assert_parity("Misc/vq_pak", &bytes);
}
