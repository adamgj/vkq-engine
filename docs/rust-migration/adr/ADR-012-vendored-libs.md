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
