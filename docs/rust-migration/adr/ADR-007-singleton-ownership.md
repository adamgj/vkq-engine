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
| `sv`/`svs` | P7 | P6 – P7 M6 (qcvm field) | P7 (**closed at M6**, sv_user.c + sv_main.c; storage is now `rust/quake-capi/src/sv_main.rs`) |
| `cl`/`cls` | P7 | P6 – P7 M7 (qcvm field) | P7 (**closed at M7**, the client stratum; storage is now `rust/quake-capi/src/cl_main.rs`) |
| `vid`, `r_refdef` | P8 | P8 sub-slices | P8 |
| `vulkan_globals` | P8 | P8 sub-slices (C-layout view) | P8 |
| `mod_known[]` | P3 (data) / P8 (vk members) | P3–P8 | P8 |
| sound globals (`shm`/`sn`, `snd_channels[]`/`total_channels`, timing, listener vectors, the 16 sound cvars, `snd_mutex`) | P4 | P4–P9: storage stays C in `snd_glue.c` for the direct C readers (menu.c cvar storage, cl_demo.c channel iteration, gl_screen timing); all *logic* is Rust, reaching the storage via `quake_c_sys` under the recursive `snd_mutex` on the main thread | P9 (host inversion) |
| mixer/sfx-registry internals (paintbuffer, scaletable, filters, underwater, `known_sfx[]`, DMA wrap counters) | — | none: Rust-owned statics from P4, never visible to C | P4 |
| net message globals (`net_message`, `msg_readcount`, `msg_badread`) | P7 | P5–P7: storage stays C (common.c, later `net_glue.c`) for the direct C readers (cl_parse.c, sv_main.c, cl_demo.c incl. the `CL_Record_Signons` `net_message.data` swap); the MSG/SZ *logic* is Rust from P5 M3, reaching the storage via `quake_c_sys` | P7 (**closed at M9e**: `rust/quake-capi/src/net.rs` now defines all three as `#[no_mangle] pub static mut` under the `net` feature. `Quake/net_main.c:77`'s `net_message` is `#ifndef USE_RUST_NET`-guarded and `Quake/net_msg_glue.c` owns no storage any more; `net.h`/`common.h` keep the declarations, so every C reader is unchanged and the `-Duse_rust_net=disabled` oracle leg keeps its own copies) |
| `net_drivers[]`/`net_landrivers[]` vtables, `qsocket_t` pool (`net_activeSockets`/`net_freeSockets`), hostcache, `net_driverlevel`/`net_time` | P5 M9 | P5 M5–M9: arrays and pool stay C-owned in net_bsd.c/net_win.c/net_main.c while Rust driver functions are installed slot-by-slot and read the ambient state via `quake_c_sys` | P5 M9 (Rust-owned driver table; `net_glue.c` keeps the C-visible remainder) |
| the edict array (`qcvm->edicts`) and the progs string table | P7 | P6–P7 M9d: the arena *logic* is Rust from P6 M3, striding the edict array as an untyped arena (ADR-006) while `pr_edict_arena.c` stayed the C owner of the free list, the free-list rebuild and the known-string table | P7 (**closed at M9d**, the `pr_edict_arena` view→owner flip: `Quake/pr_edict_arena.c` leaves the build under `-Duse_rust_progs` and `rust/quake-capi/src/pr_edict_arena.rs` owns the logic. The backing memory itself stays on `Mem_Alloc`/`Mem_Free` on both sides by ADR-013 design — this row closes on *decision* ownership, not on allocator ownership) |
| ambient `qcvm` + `pr_global_struct` (the two pointers) | P7 | P6–P7: the two pointers stay C (`Quake/pr_edict_load_glue.c:49-50` under `-Duse_rust_progs`, `Quake/pr_edict_load.c:34-35` otherwise) because 14 files outside the progs sources dereference them; the VM *logic* is Rust from P6 M3, reading the ambient global exactly once per boundary entry (ADR-008) | P7 (**still open after M9e**: moving the storage needs `rust/quake-capi/cbindgen.toml` + `progs_load.rs` changes; scheduled for **M9g**) |
| progs VM internals (interpreter locals, arena field-offset handles, loader scratch, the reverse-built symbol maps' Rust-side views) | — | none: Rust-owned from each flip milestone, never visible to C | P6 |
| driver internals (loopback buffers, dgrm packet buffer/state machine, UDP socket state) | — | none: Rust-owned module state from each driver's flip milestone, never visible to C | P5 |

Five rows name a Phase 7 milestone in their "Rust-owned from" column; the milestone letters index `docs/ai/plans/rust-conversion-phase-7.md`. **M6** (`sv`/`svs`), **M7** (`cl`/`cls`), **M9d** (the edict array + string table) and **M9e** (the net message globals) have landed, so four of those windows are closed. One remains open: the ambient `qcvm`/`pr_global_struct` pointer pair, scheduled for **M9g**. Phase 7 exits only with all five closed. The original single "qcvm + edicts" row was split at M9d because its two halves close at different points — the edict-array half at the arena flip, the pointer half only once the cbindgen surface can carry Rust-owned storage.

**End state (Phase 9/10):** singletons become fields of a `Host` struct created in `main()` and passed by `&mut` (split-borrowed into subsystem structs). Remaining `static` state exists only where a C remnant requires it, listed in the Phase-10 unsafe inventory.

## Consequences

- One audited chokepoint for cross-language global access instead of scattered `extern` declarations.
- Some transition code takes a `&mut Host`-style parameter earlier than strictly needed, which is churn now but the end-state shape.
- The dual-view table above must be updated as phases land; an unlisted dual view is a review error.
