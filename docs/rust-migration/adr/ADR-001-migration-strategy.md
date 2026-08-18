# ADR-001: Hybrid incremental oxidation; c2rust as oracle only

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

vkqr-engine is ~124k LOC of C11 with a hard requirement of 100% backwards compatibility (saves, demos, netplay, mods, re-release content) and a goal of idiomatic, type-safe Rust. Three candidate strategies:

1. **Incremental in-place oxidation** — link a growing Rust staticlib into the existing build, port module-by-module, invert ownership at the end.
2. **c2rust transpile-then-refactor** — machine-translate the whole tree, then refactor toward idiomatic Rust.
3. **Clean-room rewrite** — new Rust engine matching behavior (the Richter/Seismon approach).

Evidence: Immunant's c2rust translation of ioquake3 works but produces raw-pointer, `unsafe`-saturated, non-idiomatic code; refactoring such output fights both the original design and transpiler artifacts, and the intermediate states are unshippable. Clean-room Rust Quake engines remain protocol-15-only and incomplete after years, and would discard vkqr-engine's accumulated compatibility fixes. Incremental oxidation keeps a shippable, testable engine at every step and lets each ported module be differentially verified against the live C implementation.

## Decision

Adopt **hybrid incremental oxidation**:

- A Cargo workspace (`rust/`) builds one `staticlib` linked into `vkquake` by Meson. Modules port bottom-up; each lands behind a time-boxed `-Duse_rust_<module>` Meson switch for A/B differential testing, after which the C file is deleted.
- Ownership inverts in Phase 9: `main()` moves to Rust; remaining C becomes a static library.
- **c2rust output is never merged or linked into shipping binaries.** It lives in `tools/c2rust-oracle/` and serves only as (a) a reference for ambiguous C semantics and (b) a third implementation for differential fuzzing of the VM interpreter and physics.
- The pure-C build remains green in CI through Phase 9 as the reference oracle for all differential tests.

## Consequences

- Every phase is shippable; compatibility regressions are caught at module granularity by the harness (ADR-019).
- Cost: dual-build maintenance until Phase 9, FFI shim churn, and discipline required to delete C promptly (switches are time-boxed to prevent permanent dual implementations).
- The type-safety requirement is met by construction (hand-ported idiomatic code) rather than retrofitted onto transpiler output.
