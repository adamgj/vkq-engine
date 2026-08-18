# Architecture Decision Records — vkqr-engine Rust Migration

Each ADR captures one decision: its context, the decision itself, and its consequences. Decisions marked **(compat exception)** document a deliberate deviation from Rust best practice made to preserve backwards compatibility; code implementing them carries `// COMPAT:` comments linking back here. Decisions marked **(user decision)** were confirmed with the project owner.

Statuses: **Accepted**, **Proposed**, **Superseded by ADR-NNN**.

| ADR | Title | Status |
|---|---|---|
| [001](ADR-001-migration-strategy.md) | Hybrid incremental oxidation; c2rust as oracle only | Accepted |
| [002](ADR-002-c-not-cpp-fallback.md) | Fallback native modules are C, not C++ | Accepted |
| [003](ADR-003-dependency-policy.md) | Third-party crate policy (audit/deny gates, licenses, lockfile, MSRV) | Accepted |
| [004](ADR-004-unsafe-policy.md) | Unsafe-code policy | Accepted |
| [005](ADR-005-printf-float-formatter.md) | C-printf-compatible float formatter **(compat exception)** | Accepted |
| [006](ADR-006-edict-arena.md) | Edict/progs memory as an untyped arena with typed accessors **(compat exception)** | Accepted |
| [007](ADR-007-singleton-ownership.md) | Global singleton ownership during transition; `Host` struct end state | Accepted |
| [008](ADR-008-ambient-qcvm.md) | Ambient `qcvm` at the C boundary; explicit `&mut QcVm` internally **(compat exception)** | Accepted |
| [009](ADR-009-error-handling.md) | `Result`/`HostError` replaces setjmp/longjmp; no-unwind FFI rules | Accepted |
| [010](ADR-010-determinism-policy.md) | Determinism = per-platform parity with the C build **(compat exception)** | Accepted |
| [011](ADR-011-ffi-tooling.md) | bindgen + cbindgen + hand-mirrored `repr(C)` ABI structs | Accepted |
| [012](ADR-012-vendored-libs.md) | Vendored-library replacement map | Accepted |
| [013](ADR-013-allocator.md) | Single shared mimalloc allocator across the language boundary | Accepted |
| [014](ADR-014-audio-codecs.md) | Audio codecs remain C behind the codec vtable | Accepted |
| [015](ADR-015-renderer-port-then-modernize.md) | Renderer: port-then-modernize; ash over vulkano/wgpu | Accepted |
| [016](ADR-016-task-system.md) | Task system deferred to Phase 8; crossbeam-deque design | Accepted |
| [017](ADR-017-sdl-policy.md) | Keep SDL2 + SDL3 dual support **(user decision)** | Accepted |
| [018](ADR-018-dropped-features.md) | Dropped: IPX, makefile-only codecs, MSVC solution **(user decision)** | Accepted |
| [019](ADR-019-verification-architecture.md) | Verification architecture and C-build retirement criteria | Accepted |

## Template

```markdown
# ADR-NNN: Title

**Status:** Proposed | Accepted | Superseded by ADR-NNN
**Date:** YYYY-MM-DD
**Tags:** (compat exception) | (user decision) | —

## Context
What forces are at play; what the C code does today; why a decision is needed.

## Decision
The decision, stated actively and specifically.

## Consequences
What becomes easier, what becomes harder, what must be monitored, when to revisit.
```
