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
