# ADR-018: Dropped features — IPX, makefile-only codecs, MSVC solution

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** (user decision)

## Context

Three legacy pieces impose ongoing cost on the migration with little or no user value. The project owner approved dropping all three; everything else in the engine is preserved.

## Decision

Removed in Phase 0, in C, before any porting:

1. **IPX networking** (`net_wipx.c`, its registration in `net_win.c`, `-ipxport` plumbing). Windows-only Winsock IPX; modern Windows has not shipped a usable IPX stack in roughly two decades, and keeping it would force a permanent Windows-only FFI island in `quake-net`. UDP IPv4/IPv6 (including LAN discovery and rcon) is unaffected.
2. **Makefile-only music codecs** (`snd_mikmod.c`, `snd_xmp.c`, `snd_modplug.c` and their Makefile hooks). They are absent from the primary Meson build already; WAV/FLAC/MP3/Vorbis/Opus/UMX all remain (ADR-014).
3. **The MSVC solution** (`Windows/VisualStudio/` — `vkquake.sln` + project files). Meson with clang-cl (already exercised in CI) becomes the sole Windows build, avoiding a third build system that would each need Cargo integration. The `bintoc`/`mkpak` native tools keep building under Meson until `xtask` replaces them in Phase 8.

## Consequences

- `quake-net` targets exactly two socket families (IPv4/IPv6) on all platforms — one driver implementation, no Windows special case beyond Winsock init.
- Windows developer workflow moves to Meson/ninja (documented in the readme); Visual Studio users can still use it as an editor/debugger against the Meson build.
- Tracker-music playback via mikmod/xmp/modplug is gone; content relying on it (rare; UMX→MOD content was the main consumer and UMX support remains for its other codecs) loses those decoders. If demand appears, a Rust tracker crate can be evaluated later under ADR-003.
