# ADR-013: Single shared mimalloc allocator across the language boundary

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The engine replaced Quake's zone/hunk allocators with `mem.c` (`Mem_Alloc`/`Mem_Free`, zeroed by default, `TEMP_ALLOC` stack-or-heap macros) backed by vendored mimalloc 3.4.5. During the migration, buffers routinely cross the language boundary (a Rust parser allocates model data that C rendering later frees, or vice versa). Freeing memory with a different allocator than allocated it is undefined behavior — the classic mixed-language failure mode.

## Decision

- **One allocator on both sides:** the vendored mimalloc build serves C (via `mem.c` as today) *and* Rust — `#[global_allocator]` is a `libmimalloc-sys`-based allocator bound to the **same compiled mimalloc** (not a second copy; the sys crate links against the in-tree build).
- **Boundary rule:** any allocation whose ownership crosses the language boundary is made and released through the `Mem_*` C API (shimmed into Rust). Rust-internal allocations use normal Rust types; they never cross raw.
- Zeroing semantics (`Mem_Alloc` zeroes; `Mem_AllocNonZero` doesn't) and the `TEMP_ALLOC` per-thread stack budget behavior are preserved in the Rust shims.
- ASan runs on mixed builds in CI to catch violations of the boundary rule.
- **Revisit at Phase 10:** once no ownership crosses the boundary, choose between keeping mimalloc (perf) or dropping to the system/Rust default allocator (less vendored C). That decision closes or amends this ADR.

## Amendment (Phase 2): hand-declared `mi_*` binding instead of libmimalloc-sys

`libmimalloc-sys` was rejected at implementation time: it compiles and links
its **own** mimalloc copy, which is exactly the second-allocator situation
this ADR forbids, and its build overrides for an external library don't fit
how the engine vendors mimalloc (amalgamated into `mem.c`'s translation unit
via `#include "mimalloc/static.c"`; the `mi_*` symbols keep external
linkage). Instead:

- `quake-c-sys::mi` hand-declares `mi_malloc_aligned`/`mi_zalloc_aligned`/
  `mi_realloc_aligned`/`mi_free`, and `quake_capi::alloc::EngineMiMalloc`
  implements `GlobalAlloc` over them — zero new crates.
- The `#[global_allocator]` sits behind the `engine-alloc` cargo feature,
  set only by the Meson build: plain `cargo test`/`cargo build` binaries
  don't link `mem.o`, so the symbols don't exist there and tests run on the
  default allocator.
- `mem.c` auto-selects `USE_MI_MALLOC` on all three supported platforms; a
  hypothetical `USE_HELGRIND`/`USE_CRT_MALLOC` C build combined with
  `-Duse_rust` would fail to link the `mi_*` symbols — loudly, by design.
- The `TEMP_ALLOC` counterpart is `quake_util::scratch::ScratchBuf` (inline
  capacity first, heap spill past it). The C macros' per-thread alloca
  budget only bounds C stack usage and is not observable across the
  boundary, so the Rust type does not mirror the thread-local counter.

## Consequences

- Cross-language alloc/free is sound by construction; a whole class of transition bugs is designed out.
- The vendored mimalloc (~25k LOC of C) persists at least until Phase 10 — accepted as an ADR-002 native remnant with a scheduled revisit.
- Allocation performance characteristics stay identical to the current engine throughout the migration (no perf-regression noise from allocator changes while porting).
