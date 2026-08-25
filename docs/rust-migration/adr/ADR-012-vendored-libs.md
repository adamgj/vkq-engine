# ADR-012: Vendored-library replacement map

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The tree vendors several C libraries compiled in-place: miniz (zip/deflate — used for `.kpf` localization archives and embedded-pak compression), lodepng (PNG decode/encode), stb_image/stb_image_write/stb_image_resize (image decode/encode/resize), jsmn (tolerant JSON for MD5-model metadata), and mimalloc (allocator — see ADR-013). Replacing each with a Rust crate trades vendored C for maintained safe code, but any change in *parse acceptance* or output bytes is a compatibility break.

## Decision

Per-library, gated on evidence:

| Vendored | Replacement | Gate |
|---|---|---|
| miniz (zip read: `.kpf`) | `zip` crate (or `flate2`+`miniz_oxide`) | Golden test: byte-identical extraction + identical accept/reject over the kpf corpus and zip fuzz corpus |
| miniz (deflate: embedded pak) | `miniz_oxide` compression in `xtask` | Decompressed payload identical (compressed bytes may differ — only the payload is observable) |
| stb_image / lodepng (PNG/JPG/TGA **decode**) | `png` / `image` crates behind the `Image_Load*` seam | Pixel-exact decode over the full texture corpus + differential fuzzing vs C decoders |
| lodepng (PNG **encode**, screenshots) | keep lodepng (C) until/unless parity accepted | Screenshots are user output, not engine input; low priority — retained C remnant is acceptable (ADR-002) |
| stb_image_resize | Rust resize (port or crate) | Output feeds texture upload; require pixel-exact or explicitly accept a visual-only tolerance (renderer-side, ADR-015) |
| jsmn / `json.c` | **hand-port, no serde_json** | MD5-metadata acceptance must not change; jsmn's tolerant, non-validating behavior is the spec |
| mimalloc | keep (ADR-013) | — |

General rule: the C implementation stays (behind the existing seam) until its replacement's gate is green in CI; swaps are per-library PRs, revertible.

## Consequences

- Parser-facing attack surface moves to memory-safe code where it matters most (network/file input), without silent acceptance drift.
- A small list of C remnants (lodepng encoder at minimum) persists intentionally; tracked in Phase 10.
- jsmn's hand-port is extra work versus serde_json but preserves mod-content acceptance exactly.

## Amended (Phase 3 M8, 2026-08-24)

The stb_image decode swap landed with the gate applied per format:

- **TGA**: hand-ported in `quake-image::tga` (no crate), bit-exact vs stb
  including reject reasons and out-param write points.
- **PNG**: `png` crate behind a hand-ported acceptance layer
  (`quake-image::png_stb` reimplements stbi__parse_png_file's chunk walk and
  zlib-header checks bit-for-bit; the crate performs inflate/defilter/expand
  only, checksums off to match stb). Pixel-exact over the gate corpus.
  CgBI/iPhone PNGs route to the retained C stb (`Image_DecodeSTBMem`), so
  their color-mangled acceptance stays exactly stb's.
- **JPEG (compat exception, owner decision 2026-08-24)**: the pixel-exact
  requirement of this ADR's table is **waived for JPEG only** — stb's
  fixed-point IDCT/upsampler cannot be matched bit-for-bit by another
  implementation, and the owner ruled "pixel exact with jpeg rendering is
  not critical". `zune-jpeg` ships under a relaxed gate enforced by
  `image_crate_differential`/`image_real_assets`/`formats_corpus`:
  accept/reject parity, identical dimensions, and a pinned per-channel
  delta (≤ 8; measured 0 on the synthetic matrix and 3–4 on the two
  re-release depot photographs). Textures are renderer input (pixels, not
  simulation state), so no golden or state-hash surface moves.
- **PNG encode (screenshots) stays lodepng** and JPEG/TGA *encode* stay
  stb_image_write, unchanged, per the table above.
- The C decoder remains compiled: the streaming `Image_DecodeSTB` under
  `-Duse_rust_image=disabled` (oracle), and the in-memory
  `Image_DecodeSTBMem` in every configuration (CgBI fallback + per-format
  revert lever).
