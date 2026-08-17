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
