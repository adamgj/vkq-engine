# ADR-003: Third-party crate policy

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The project allows Cargo dependencies but requires them to be widely used, actively supported, and free of open high/critical CVEs. vkQuake is GPLv2+, constraining acceptable licenses. Supply-chain risk must be managed for a codebase that parses untrusted network and file input.

## Decision

- **CI gates:** `cargo audit` (RustSec advisory DB) and `cargo deny check` run on every PR. Any advisory of high/critical severity on the dependency graph fails the build; lower severities require a tracked issue with a remediation plan.
- **Licenses:** `cargo deny` enforces an allowlist of GPLv2-compatible licenses (MIT, Apache-2.0, BSD-2/3, Zlib, MPL-2.0, ISC, Unicode-DFS). Copyleft additions beyond that require explicit review.
- **Lockfile:** `Cargo.lock` is committed. Dependency updates are deliberate PRs, not implicit.
- **MSRV:** pinned in `rust-toolchain.toml` and workspace metadata; bumps are explicit PRs.
- **Introduction bar:** each new *direct* dependency requires a short review note in its PR: maintenance status (recent releases/commits, bus factor), download/user base, transitive dependency footprint, and alternatives considered. Prefer std or small hand-written code over a dependency for trivial functionality.
- **Expected direct dependencies** (reviewed at introduction, not grandfathered): `ash`, `sdl2`, `sdl3`, `crossbeam-deque`/`crossbeam-utils`, `zip` or `flate2`+`miniz_oxide`, `png`/`image`, `libmimalloc-sys`, `bitflags`, `libc`; build/dev-only: `bindgen`, `cbindgen`, `libfuzzer-sys`, `criterion`, `proptest`, `loom`.

## Consequences

- CVE exposure is monitored continuously rather than at release time; the "no open high/critical CVE" requirement is enforced mechanically.
- Some conveniences are rejected for footprint reasons; small utility code is written in-tree instead.
- License compliance is automatic; GPLv2+ distribution remains clean.
