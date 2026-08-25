# ADR-017: Keep SDL2 + SDL3 dual support

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** (user decision)

## Context

The C engine supports both SDL2 and SDL3 (`USE_SDL3` compile switch; per-file splits `in_sdl2.c`/`in_sdl3.c`, `snd_sdl.c`/`snd_sdl3.c`; a threading-name compat shim in `quakedef.h`). SDL3 is required on Windows and preferred elsewhere; SDL2 remains a fallback for Linux/macOS systems without SDL3. Dropping SDL2 would roughly halve the platform-layer port. The project owner chose to **keep dual support**.

## Decision

- `quake-platform` supports both, selected by mutually exclusive cargo features (`sdl2`, `sdl3`) mapped from the existing Meson option, using the `sdl2` and `sdl3` crates respectively.
- Structure mirrors the C approach that already works: a thin common layer (the Rust equivalent of `in_sdl.c`'s shared logic and the threading-name shim) over two backend modules, so version-specific code stays as small as the C splits are today.
- Windows remains SDL3-only (as in C). CI builds both features on Linux and macOS (matching the existing macOS CI matrix).
- The scancode→Quake-key table (`in_sdl.h`) ports once with per-backend mappings.
- Revisit when upstream vkqr-engine or its distribution targets drop SDL2; this ADR then flips to SDL3-only with a one-phase deprecation window.

## Amendment (Phase 4 M9, 2026-08-25)

The audio backend adopted the `sdl3` crate (v0.18.4; MIT, with `sdl3-sys` Zlib — ADR-003 review in the M9 commit) ahead of the platform phase: `quake-platform::snd_sdl3` ports `snd_sdl3.c` over the crate's sys layer, and quake-capi exports `SNDDMA_*` under `snd`+`sdl3`. The **SDL2 audio backend stays C** for now: SDL2 development libraries are absent from the current dev and CI environments, so a Rust port could not be built or verified anywhere; it follows once a use_rust+SDL2 CI leg exists (tracked for the platform phase). Meson keeps compiling `snd_sdl.c` under `-Duse_rust_snd` when SDL2 is selected.

## Consequences

- Linux/macOS users on SDL2-only systems keep working — the compatibility-first principle extended to the platform layer.
- Cost accepted: two backend code paths in `quake-platform`, double CI legs for the platform layer, and tracking two binding crates under the ADR-003 dependency policy (the `sdl2` crate's maintenance status must be monitored — it is in maintenance mode as SDL3 adoption grows).
