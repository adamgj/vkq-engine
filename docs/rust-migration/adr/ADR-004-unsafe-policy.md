# ADR-004: Unsafe-code policy

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The migration requires large amounts of FFI (transition period), a Vulkan renderer (inherently unsafe at the binding level), a lock-free task scheduler, and an untyped edict arena (ADR-006). Unsafe code cannot be avoided, but it can be concentrated, audited, and shrunk over time.

## Decision

- Pure crates — `quake-math`, `quake-util`, `quake-formats` (parsers), `quake-cvar`, `quake-image` (orchestration), `quake-fs` (logic) — carry `#![forbid(unsafe_code)]`.
- All other crates: `#![deny(unsafe_op_in_unsafe_fn)]` and `clippy::undocumented_unsafe_blocks` as an error — every `unsafe` block has a `// SAFETY:` comment stating the invariant and why it holds.
- Unsafe is **concentrated** in five places: `quake-c-sys` (FFI imports), `quake-capi` (FFI exports), `quake-render` (ash-level Vulkan), `quake-tasks` (bounded scheduler internals, loom-tested), and the `EdictArena` module of `quake-progs`.
- SIMD (`std::arch`) intrinsic blocks in hot paths (culling) are permitted with SAFETY comments and scalar reference implementations used by tests.
- Workspace lints deny warnings in CI (parity with the C build's `werror=true`).
- An "unsafe inventory" (grep-based count per crate) is tracked; Phase 10 includes a review pass to minimize what remains.

## Consequences

- Soundness review effort focuses on five known locations rather than the whole tree.
- Some transition-period code is more verbose (status-code shims, accessor funnels) than a permissive style would be; this is accepted as the cost of auditability.
- `forbid(unsafe_code)` crates give reviewers a hard guarantee for the majority of ported logic.

## Amended (Phase 5 M1, 2026-08-25)

`quake-net` is added as the **sixth concentrated location**, in a bounded
form: the crate is `#![deny(unsafe_code)]` crate-wide with a single
`#[allow(unsafe_code)]` `sys` module planned for the M7 UDP landriver (the
fd↔socket boundary: socket2 raw-fd interop, `getifaddrs`/adapter
enumeration, Winsock init). Everything else in the crate — MSG/SZ
serialization, the dgrm state machine, CCREQ/CCREP parsing, demo file IO,
loopback — stays unsafe-free and mock-testable. This mirrors the
`EdictArena`-module precedent: unsafe confined to one named module inside an
otherwise-denying crate. The `extern "C"` driver entry points themselves
live in `quake-capi` as usual (ADR-011). *(Landed at M7b as
`quake_net::udp::sys`, unix-only until the net_wins.c flip.)*
