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

## Amended (Phase 6 M1, 2026-08-27)

`quake-progs` follows the same bounded shape: the crate is
`#![deny(unsafe_code)]` crate-wide with a single `#[allow(unsafe_code)]`
`arena` module. That module is the crate's whole untyped-memory island: the
ADR-006 edict buffer (the original motivating case for the "concentrated
location" list), the progs string table, and — from Phase 6 M3 — `VmRaw`, the
borrow-free view over the C-owned `qcvm_t`'s lumps, global block and stacks.
The interpreter in `exec` is safe code on top of it. The loader, interpreter control
flow, `ED_Write`/`ED_Parse*` and the builtin bodies stay unsafe-free and
fuzzable; the crate deliberately does not depend on `quake-c-sys`, so nothing
in it can reach an engine global directly. The `extern "C"` VM entry points
and the ambient-`qcvm` resolution live in `quake-capi` as usual (ADR-011).

## Amended (Phase 10 M2, 2026-09-16): inventory made reproducible; second `quake-progs` island

- The "unsafe inventory (grep-based count per crate)" is now
  `scripts/unsafe_inventory.py`, which writes
  [`unsafe-inventory.md`](../unsafe-inventory.md) and runs with `--check` in
  CI. It encodes this ADR's tier table (forbid / deny-with-listed-modules /
  open) and fails when a crate's `unsafe_code` attribute or its
  `allow(unsafe_code)` module set departs from it, so the policy is enforced
  rather than recorded.
- **`quake-progs` has two islands, not one.** Phase 6 added
  `quake_progs::image` (the raw view over a `progs.dat` image while it is
  loaded) beside `arena`, documented in the crate but never here. It is the
  same shape of problem as the arena — untyped C memory whose layout is
  decided at runtime — and is accepted as the crate's second and last
  `allow(unsafe_code)` module. The Phase 6 amendment's "single" is superseded
  by this list: `arena`, `image`.
- `quake-net`'s island is the inline `mod sys` in `src/udp.rs` (not a
  separate file), as landed at Phase 5 M7b.
- `quake-tasks` was named a concentrated location above but landed
  `#![forbid(unsafe_code)]` (Phase 8; its scheduler unsafe lives in
  `quake-capi`). The tier table records it as a pure crate.
- Review outcome at this cut (6 791 tokens): `quake-capi` 5 350 (79 %,
  spread over 94 of 98 files — the dual-view accessors and `extern "C"`
  exports that Phase 10 M6 removes), `quake-platform` 406 (Phase 9 SDL/OS
  calls), `quake-render` 323, `quake-c-sys` 286 (246 in `unsafe extern`
  blocks), `quake-ctest` 207 (harness only), `quake-progs` 139,
  `quake-net` 44; `quake-types` 36 are `unsafe extern "C" fn` pointer
  *types* under `forbid`. Nothing outside the concentrated locations. The
  minimization target is the M6/M7 shim removal, which this inventory will
  measure; no unsafe was hand-removed at this cut because every remaining
  site serves a C caller that still exists.
