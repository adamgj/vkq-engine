# ADR-007: Global singleton ownership during transition; `Host` struct end state

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The engine is anchored on global singletons read/written across modules: `cl`/`cls` (client state), `sv`/`svs` (server state), `vulkan_globals` (~800-line renderer state), `vid`, `r_refdef`, `mod_known[]`, `com_searchpaths`. During an incremental migration both languages must see consistent state; at the end, Rust code should not be built on `static mut`.

## Decision

**Transition:** each singleton stays owned by the language that owns its subsystem's phase. While C-owned, Rust access goes exclusively through one audited module (`quake_c_sys::globals`) whose accessor functions carry `// SAFETY:` comments naming the synchronization argument (single-threaded host frame, or the specific task-graph phase that has exclusive access). When a phase flips a subsystem to Rust, the Rust side owns the state and exports a C-layout view (`#[no_mangle] static` or accessor fns) **only while** remaining C still touches it.

**Dual-view windows** (both languages seeing one struct) are minimized by roadmap ordering and enumerated here as they open/close:

| Singleton | C-owned until | Dual-view window | Rust-owned from |
|---|---|---|---|
| `com_searchpaths` etc. (fs) | P2 | P2 only | P2 |
| `sv`/`svs` | P7 | P6–P7 (qcvm field) | P7 |
| `cl`/`cls` | P7 | P6–P7 (qcvm field) | P7 |
| `vid`, `r_refdef` | P8 | P8 sub-slices | P8 |
| `vulkan_globals` | P8 | P8 sub-slices (C-layout view) | P8 |
| `mod_known[]` | P3 (data) / P8 (vk members) | P3–P8 | P8 |
| sound globals (`shm`/`sn`, `snd_channels[]`/`total_channels`, timing, listener vectors, the 16 sound cvars, `snd_mutex`) | P4 | P4–P9: storage stays C in `snd_glue.c` for the direct C readers (menu.c cvar storage, cl_demo.c channel iteration, gl_screen timing); all *logic* is Rust, reaching the storage via `quake_c_sys` under the recursive `snd_mutex` on the main thread | P9 (host inversion) |
| mixer/sfx-registry internals (paintbuffer, scaletable, filters, underwater, `known_sfx[]`, DMA wrap counters) | — | none: Rust-owned statics from P4, never visible to C | P4 |

**End state (Phase 9/10):** singletons become fields of a `Host` struct created in `main()` and passed by `&mut` (split-borrowed into subsystem structs). Remaining `static` state exists only where a C remnant requires it, listed in the Phase-10 unsafe inventory.

## Consequences

- One audited chokepoint for cross-language global access instead of scattered `extern` declarations.
- Some transition code takes a `&mut Host`-style parameter earlier than strictly needed, which is churn now but the end-state shape.
- The dual-view table above must be updated as phases land; an unlisted dual view is a review error.
