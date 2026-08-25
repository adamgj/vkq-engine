//! PCX decode fuzzer (Phase 3 M7, D11 / AC3): drives the pure quake-image
//! PCX decision layer the `Image_DecodePCX` shim is built on — the exact
//! accept/reject predicates that decide, bit-for-bit, what the Rust engine
//! does. The true C-via-FFI differential over real and malformed PCX assets
//! lives in `quake-ctest`'s `image_differential` and the `formats_corpus`
//! gate; this target is coverage-guided exploration of the header/decode
//! decision that must never panic and must keep every write in bounds.
//!
//! Input layout: the first 128 bytes are the pcxheader_t (fuzzed), the last
//! 768 are the tail palette, and the middle is the RLE stream. A short input
//! returns early (the C decoder's own out-of-resource behavior is UB, a
//! documented divergence, not something to compare here).

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_image::pcx;

fuzz_target!(|data: &[u8]| {
    if data.len() < pcx::HEADER_SIZE + pcx::PALETTE_SIZE {
        return;
    }

    let header = match pcx::parse_header(data) {
        Ok(h) => h,
        Err(_) => return, // a rejected header is a valid decision, no decode
    };

    // The shim allocates `alloc_size` bytes (Mem_Alloc, zero-filled) and
    // decodes into it. Reproduce that, but clamp so a hostile width*height
    // can't ask for gigabytes; a clamp only makes the buffer smaller, which
    // can only turn a would-be in-bounds write into a rejected overrun, so
    // the "never writes out of bounds" invariant still holds.
    let want = header.alloc_size;
    if want <= 0 {
        // wrapping i32 math produced a non-positive size; the C passes the
        // same value to Mem_Alloc — nothing to decode into here
        return;
    }
    let cap = (want as usize).min(16 << 20);
    let mut out = vec![0u8; cap];

    // Must return Ok or Err(NotValid) — never panic, never write past `out`
    // (Rust bounds every store; a panic here would be the bug) — and must be
    // deterministic: a second decode into a fresh zeroed buffer of the same
    // size reproduces both the status and every byte.
    let status = pcx::decode(data, &header, &mut out);
    let mut again = vec![0u8; cap];
    assert_eq!(status, pcx::decode(data, &header, &mut again));
    assert_eq!(out, again);
});
