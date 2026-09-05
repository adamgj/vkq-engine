//! Work-stealing job system (Rust migration Phase 8 M2, ADR-016).
//!
//! Port of `Quake/tasks.c`. The crate is the scheduler *core*: it owns the
//! 256-slot task table, the dependency/epoch state machine, the join
//! primitive, the worker execution loop and the `-pinnedworkers` parser, and
//! it is generic over the [`Job`] it runs. Everything C-facing -- the
//! `Task_*`/`Tasks_*` exports, the 128-byte payload copy, the thread-locals
//! behind `Tasks_IsWorker`/`Tasks_GetWorkerIndex`, thread pinning -- lives in
//! `quake-capi`, so this crate stays `forbid(unsafe_code)` (ADR-004) and the
//! loom models cover the real state machine.
//!
//! What the C callers can observe, and what is therefore preserved exactly:
//!
//! * handle encoding `index | epoch << 8` (`tasks.c` `NUM_INDEX_BITS`), never
//!   `INVALID_TASK_HANDLE`;
//! * 256 slots of which 255 are allocatable (`Tasks_Init` seeds the free ring
//!   with `MAX_PENDING_TASKS - 1` entries); `allocate` blocks, never fails;
//! * at most 16 dependents per task; `add_dependency` is a silent no-op once
//!   `before` has retired and then does *not* touch `after`'s count;
//! * `join(timeout)` returns `true` once the slot's epoch has moved past the
//!   handle's (so `true` on a stale handle) and `false` on timeout without
//!   invalidating anything;
//! * indexed tasks are chunked per worker exactly as `Task_AssignIndexedFunc`
//!   does, and fan out over `min(limit, num_workers)` workers, each walking
//!   the chunks from its own outward;
//! * with a single worker, independent tasks run in submission (FIFO) order.
//!
//! What is internal and differs from C (ADR-016 allows it): the executable
//! queue is a crossbeam injector plus per-worker deques instead of two Vyukov
//! rings; the free list is a plain FIFO; workers idle on a condvar gate after
//! a bounded spin instead of a semaphore.

#![forbid(unsafe_code)]

pub mod pinned;
mod queue;
mod scheduler;
mod sync;

pub use scheduler::{Job, Scheduler, TaskHandle, Timeout};

/// `tasks.c` -- `NUM_INDEX_BITS`: low bits of a handle holding the slot index.
pub const NUM_INDEX_BITS: u32 = 8;
/// `tasks.c` -- `MAX_PENDING_TASKS`: size of the task table.
pub const MAX_PENDING_TASKS: usize = 1 << NUM_INDEX_BITS;
/// `tasks.c` -- `MAX_DEPENDENT_TASKS`.
pub const MAX_DEPENDENT_TASKS: usize = 16;
/// `tasks.h` -- `TASKS_MAX_WORKERS`.
pub const MAX_WORKERS: usize = 32;
/// `tasks.h` -- `INVALID_TASK_HANDLE` (`UINT64_MAX`). Never produced here.
pub const INVALID_TASK_HANDLE: u64 = u64::MAX;

#[cfg(all(test, loom))]
mod loom_tests;
#[cfg(all(test, not(loom)))]
mod tests;
