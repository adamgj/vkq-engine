# ADR-002: Fallback native modules are C, not C++

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The project brief asked that pieces impossible or impractical to convert be "broken off into isolated C++ modules that the main Rust engine can link/import." The codebase, however, contains **no C++**: it is pure C11 plus one ~70-line Objective-C file (`Quake/pl_osx.m`). The pieces expected to remain native long-term are existing C code (vendored mimalloc, audio codec libraries, possibly the lodepng encoder) — all already written, tested, and battle-hardened in C.

Converting C to C++ would add a third language and toolchain to the project, introduce C++ ABI/exception/runtime concerns at the Rust boundary (Rust↔C FFI is a stable, well-understood ABI; Rust↔C++ is not), and rewrite working code for zero functional benefit.

## Decision

Pieces that are not converted to Rust remain **C**, isolated behind explicit `extern "C"` interfaces, exactly as the brief intends ("isolated native modules the Rust engine links") but without the language conversion. C++ would be introduced only if a future dependency is C++-only, and then wrapped behind a C ABI.

This is a documented deviation from the brief's literal wording, approved by keeping the brief's intent: minimal, isolated, linkable native modules.

## Consequences

- The FFI story stays single-ABI (`extern "C"`) throughout the migration.
- Expected long-term native remnants (tracked in Phase 10 of the [ROADMAP](../ROADMAP.md)): vendored mimalloc (see ADR-013), audio codec libraries (ADR-014), lodepng encoder (ADR-012), and possibly a tiny ObjC stub for macOS platform hooks.
- Each remnant must have an enumerated justification in the Phase-10 appendix to this ADR when that phase closes.

## Phase-10 appendix: surviving native code (pre-deletion cut, 2026-09-16)

Generated inventory: [`c-remnant-inventory.md`](../c-remnant-inventory.md)
(`scripts/c_remnant_inventory.py --check` in CI). This appendix is the
justified list the ROADMAP's Phase 10 exit asks for, cut **before** the
Phase 9 soak-exit deletion PR; it is re-cut at Phase 10 M10 once the
`oracle` and `glue` rows are gone. The categories that survive the deletion
PR and are *not* on a port list are the remnants proper:

| Remnant | Lines | Reached through | Why it stays | Decided by |
| --- | ---: | --- | --- | --- |
| vendored mimalloc 3.4.5 (`Quake/mimalloc/`) | 24 743 | `mem.c` (`#include "mimalloc/static.c"`), `quake-c-sys::mi` | one allocator on both sides while any allocation crosses the boundary; the revisit criterion is recorded in ADR-013 and can only be evaluated after the deletion PR | [ADR-013](ADR-013-allocator.md) |
| audio codec bridges (`snd_flac.c`, `snd_mpg123.c`/`snd_mp3.c`, `snd_opus.c`, `snd_vorbis.c`) | 1 470 | `quake-snd` `Codec` trait over FFI | the only Rust replacement named in ADR-014 (Symphonia) is MPL-2.0, which [ADR-003](ADR-003-dependency-policy.md) forbids; decoders stay C behind the trait, individually swappable if a permissive decoder is adopted by a new ADR | [ADR-014](ADR-014-audio-codecs.md) |
| lodepng encoder (`lodepng.c/.h`, host `image.c`) | 8 758 | `Image_WritePNG` | PNG screenshot bytes were never accepted as a parity surface; keeping the encoder is cheaper than proving one | [ADR-012](ADR-012-vendored-libs.md) |
| stb_image decode fallback (`stb_image.h`, host `image_stb.c`) | 7 990 | `Image_DecodeSTBMem` | formats `quake-image` deliberately does not decode (CgBI PNG, exotic JPEG) route through the C decoder as the acceptance oracle | ADR-012 |
| stb_image_write / stb_image_resize (`image.c`, `gl_texmgr*.c`) | 3 341 | TGA/JPEG writer, mipmap resize | encode/resize output is user-facing, not sim-observable; same reasoning as lodepng | ADR-012 |
| harness TUs (`harness.c`, `harness_render.c`) | 966 | `-Dtrace` / harness builds | the differential harness is defined against the C oracle build; a Rust twin is the harness's own Phase-10 item, not a remnant | [ADR-019](ADR-019-verification-architecture.md) |
| build tools (`Shaders/bintoc.c`, `Misc/vq_pak/mkpak.c`) | 317 | Meson `native: true` executables | needed by the C build only; the Rust build already uses `xtask` (Phase 8 M11); deleted with the C build | — |

Not a remnant, but still native at this cut and on a port list:

- `shared` engine C (19 TUs, 23 421 lines): `pr_ext.c`, `model_parse.c`,
  `gl_model.c`, `pr_cmds.c`, `common.c`, `pr_edict.c`, `net_main.c`,
  `palette.c`, `net_loop.c`, `net_win.c`/`net_bsd.c`, `pr_trace.c`,
  `q_thread_sdl.c`, `steam_api.c`, `cd_null.c`, `snd_sdl.c` (SDL2 legs only,
  ADR-017), plus the three vendored host TUs `mem.c`, `image.c`,
  `image_stb.c`. Ported at Phase 10 M8 (M9 for `mem.c`).
- `glue` TUs (55, 21 199 lines): Host_Guard trampolines and C-owned globals
  (ADR-007/ADR-009); removed TU by TU at Phase 10 M6.
- `miniz` and `jsmn` are included only by C-oracle TUs (`common_fs.c`,
  `json.c`) and leave with them in the deletion PR.
- No Objective-C: `pl_osx.m` was never in the Meson build and the SDL
  clipboard path covers macOS (Phase 9 M4). The "tiny ObjC stub" anticipated
  above is not needed.
