# ADR-016: Task system deferred to Phase 8; crossbeam-deque design

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

`tasks.c` (702 lines + `atomics.h`) implements a work-stealing job system: up to 32 SDL-thread workers with optional CPU pinning (`-pinnedworkers`), lock-free-ish ring queues, task handles with generation counters, dependency graphs (`Task_AddDependency`), indexed tasks (parallel-for), and `Task_Join` with timeout. Its dominant clients are the renderer's per-frame task graph (~20 tasks, 6-way parallel command recording) and parallel model loading. It is small enough to port early — but it is the concurrency spine: a subtle scheduling bug would destabilize everything above it, and porting it buys zero compatibility (no sim-observable behavior).

## Decision

- **Defer the port to Phase 8**, immediately before the renderer (its main client), so the new scheduler and its biggest consumer are validated together.
- Design: work-stealing on **`crossbeam-deque`** (injector + per-worker deques + stealers), preserving the public API semantics exactly via C shims during transition: `Task_Allocate`/`Task_AssignFunc`/`Task_AssignIndexedFunc`/`Task_AddDependency`/`Task_Submit`/`Task_Join(timeout)`, handle generation counters, `TASKS_MAX_WORKERS 32`, worker-count selection from logical cores, and thread-affinity pinning (via `libc` on the platforms where the C code supports it today).
- Dependency-graph semantics (a task runs when all dependencies complete; indexed tasks fan out then join) are preserved; internal queue mechanics may differ (they are not observable).
- Validation: `loom` models of the scheduler core, TSan on the full engine (replacing the current helgrind workflow), plus an in-engine stress test mirroring `TestTasks_f`. Worker-utilization benchmarks compare against C on the frame graph.
- Until Phase 8, Rust code scheduled *onto* C task workers (parallel model loading in Phase 3) must be `Send`-safe pure functions over byte slices — no Rust-side thread-local assumptions.

## Consequences

- Concurrency risk is spent once, in the phase equipped to validate it, instead of being carried from Phase 2 onward.
- Phase 3–7 Rust code runs on C worker threads; the `Send`-safety constraint is enforced by the module structure (parsers are pure).
- The `atomics.h` dual implementation (C11/Interlocked) retires with `tasks.c`; Rust `std::sync::atomic` replaces it wholesale.

## Amended (Phase 8 M2, 2026-09-05) — as implemented

The port landed as the `quake-tasks` crate plus the C ABI shim in
`quake-capi/src/tasks.rs`, behind `-Duse_rust_tasks` (task plan
`docs/ai/plans/rust-conversion-phase-8.md`, decision D3). Four statements
above did not survive contact with the repository; the points below replace
them and the rest of the ADR stands.

- **`atomics.h` does not retire with `tasks.c`.** Fifteen C files and seven
  headers outside the task system still use it (`gl_rmisc.c` alone about 60
  times, plus `gl_rmain.c`, `r_world.c`, `r_brush.c`, `gl_heap.c`,
  `gl_model.c/.h`, `host_cmd(_glue).c`, `r_part*(_glue).c`, `gl_sky.c`,
  `gl_warp.c`, `model_parse.c`, `glquake.h`, `quakedef.h`). It retires with
  the last C renderer file and its deletion is recorded with the other
  Phase 8 deferrals (task plan NG3).
- **TSan is net-new, not a replacement.** No helgrind workflow exists in CI
  (`helgrind.supp` only appears as a paths exclusion; the `USE_HELGRIND`
  annotations in `tasks.c` served a local `.vscode` task). The `rust.yml`
  `tsan` job (nightly, `-Zsanitizer=thread`, `-Zbuild-std`) over
  `quake-tasks` and the `loom` job are the sanitizer coverage; an in-engine
  TSan run of the mixed build is a Linux-only follow-up, not part of M2.
- **Pinning goes through the C `Sys_PinCurrentThread`, not `libc`.** The
  platform code (`sys_sdl_win.c`, `sys_sdl_unix.c`; returns false on
  macOS/BSD) stays C until Phase 9, so the shim calls it through a hand
  extern in `quake-c-sys/src/tasks.rs`. No crate was added for affinity.
- **Workers are Rust `std::thread`s, not SDL threads.** They are named
  `Task_Worker_<i>` through `std::thread::Builder`, never joined (the C
  workers are detached too) and never host a `setjmp` frame; `Sys_Error` on
  a worker already takes the non-`longjmp` path behind its `Tasks_IsWorker()`
  gate, so ADR-009 holds without a wrapper. `tasks.c` never called SDL from
  a worker, so no SDL per-thread state is lost.
- **Unsafe-free core.** `quake-tasks` is `#![forbid(unsafe_code)]` and
  generic over a `Job` trait; the only `unsafe` is the FFI call and the
  128-byte payload copy in `quake-capi` (ADR-004). The queue is
  `crossbeam-deque` in the staticlib and a mutex-guarded `VecDeque` under
  `cfg(loom)`, so the loom models exercise the real dependency/epoch state
  machine. Idling uses a per-slot `Mutex`/`Condvar` pair after the bounded
  spin (`WAIT_SPIN_COUNT` 100, as in C), so the `crossbeam-utils` `Parker`
  was not adopted.
- **Observable versus internal, recorded.** Handle encoding
  (`index | epoch << 8`), the 256-slot table, the payload copy, the
  16-dependent cap, the retired-`before` no-op in `Task_AddDependency`, the
  `Task_Join` timeout semantics and the `Tasks_NumWorkers()` selection
  (`CLAMP (1, cores, 32)`, overridden by a valid `-pinnedworkers` list) are
  preserved exactly. Slot reuse order differs (C recycles through an MPMC
  ring, Rust through a free list); the only handle held across frames,
  `prev_end_rendering_task`, is used through epoch-safe calls, so the
  difference is unobservable.
  A null `payload` zero-fills the 128-byte copy where C leaves the slot's
  stale bytes; no caller passes NULL (task plan RA16), so this too is
  unobservable.
