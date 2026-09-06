//! The task table, dependency/epoch state machine, join and worker loop
//! (`tasks.c` `Task_*`/`Tasks_*` minus the C ABI, which `quake-capi` adds).

use crate::queue::{ExecQueue, Queue};
use crate::sync::{
    lock, spawn_named, spin_hint, thread, Arc, AtomicBool, AtomicU32, Condvar, Mutex, Ordering,
    RwLock,
};
use crate::{MAX_DEPENDENT_TASKS, MAX_PENDING_TASKS, MAX_WORKERS, NUM_INDEX_BITS};
use core::time::Duration;
use std::collections::VecDeque;

/// `tasks.c` -- `WAIT_SPIN_COUNT`: queue polls before a worker sleeps.
#[cfg(not(all(test, loom)))]
const WAIT_SPIN_COUNT: usize = 100;
#[cfg(all(test, loom))]
const WAIT_SPIN_COUNT: usize = 0;

/// The unit of work the scheduler runs. `index` is `None` for a scalar task
/// (`task_func_t`) and `Some(i)` for each index of an indexed task
/// (`task_indexed_func_t`). Indexed jobs are run concurrently from several
/// workers through the same shared reference.
pub trait Job: Send + Sync + 'static {
    fn run(&self, index: Option<u32>);
}

/// `tasks.h` -- `task_handle_t`: `slot index | epoch << NUM_INDEX_BITS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TaskHandle(u64);

impl TaskHandle {
    const INDEX_MASK: u64 = (1 << NUM_INDEX_BITS) - 1;

    fn new(index: usize, epoch: u64) -> Self {
        Self(index as u64 | (epoch << NUM_INDEX_BITS))
    }

    /// Wraps a raw `task_handle_t` value.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw `task_handle_t` value.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// `tasks.c` -- `IndexFromTaskHandle`.
    pub const fn index(self) -> usize {
        (self.0 & Self::INDEX_MASK) as usize
    }

    /// `tasks.c` -- `EpochFromTaskHandle`.
    pub const fn epoch(self) -> u64 {
        self.0 >> NUM_INDEX_BITS
    }
}

/// `Task_Join`'s `timeout`: `TASK_TIMEOUT_INFINITE` or a millisecond bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    Infinite,
    Millis(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `TASK_TYPE_NONE`: allocated, never assigned; runs nothing.
    None,
    Scalar,
    Indexed,
}

/// The parts of `task_t` written only between `allocate` and `submit` and
/// read by the workers that execute the task.
struct Body<J> {
    kind: Kind,
    job: Option<J>,
    /// `task_t::indexed_limit`.
    indexed_limit: u32,
}

/// `task_t::epoch` and the dependents list, both guarded by
/// `task_t::epoch_mutex` in C.
struct EpochState {
    epoch: u64,
    dependents: Vec<TaskHandle>,
}

struct Slot<J> {
    state: Mutex<EpochState>,
    epoch_cv: Condvar,
    body: RwLock<Body<J>>,
    remaining_dependencies: AtomicU32,
    remaining_workers: AtomicU32,
}

/// `task_counter_t`: one per (slot, worker) for indexed fan-out.
struct Counter {
    index: AtomicU32,
    limit: AtomicU32,
}

struct Inner<J> {
    num_workers: usize,
    slots: Vec<Slot<J>>,
    /// `indexed_task_counters`, indexed `worker * table_size + slot` as in
    /// C (`IndexedTaskCounterIndex`), so the counters two workers hammer
    /// during one fan-out sit a table apart rather than on one cache line.
    counters: Vec<Counter>,
    /// `free_task_queue`: FIFO of retired slot indices.
    free: Mutex<VecDeque<u32>>,
    free_cv: Condvar,
    /// `executable_task_queue`.
    queue: ExecQueue,
    /// Number of workers blocked on `idle_cv`; the gate every submission
    /// passes so a sleeping worker cannot miss a push.
    idle: Mutex<usize>,
    idle_cv: Condvar,
    stop: AtomicBool,
}

/// The scheduler. Cheap to clone (shared); worker threads hold clones.
pub struct Scheduler<J: Job> {
    inner: Arc<Inner<J>>,
}

impl<J: Job> Clone for Scheduler<J> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<J: Job> Scheduler<J> {
    /// Builds the table for `num_workers` workers (`1..=MAX_WORKERS`) with
    /// C's 255 allocatable slots. Workers are not started; see
    /// [`Scheduler::spawn_workers`].
    pub fn new(num_workers: usize) -> Self {
        Self::with_free_slots(num_workers, MAX_PENDING_TASKS - 1)
    }

    /// `Tasks_Init` with a smaller free ring, so tests can reach slot reuse
    /// (and the blocking `allocate`) without cycling 255 tasks.
    pub(crate) fn with_free_slots(num_workers: usize, free_slots: usize) -> Self {
        Self::with_table(num_workers, MAX_PENDING_TASKS, free_slots)
    }

    /// `with_free_slots` over a table of `table_size` slots, so the loom
    /// models track a handful of slots instead of 256.
    pub(crate) fn with_table(num_workers: usize, table_size: usize, free_slots: usize) -> Self {
        assert!((1..=MAX_WORKERS).contains(&num_workers));
        // A table needs at least one free slot plus the sentinel slot the
        // ring keeps empty, so a 1-slot table is not a valid configuration.
        assert!((2..=MAX_PENDING_TASKS).contains(&table_size));
        assert!((1..table_size).contains(&free_slots));
        let slots = (0..table_size)
            .map(|_| Slot {
                state: Mutex::new(EpochState {
                    epoch: 0,
                    dependents: Vec::with_capacity(MAX_DEPENDENT_TASKS),
                }),
                epoch_cv: Condvar::new(),
                body: RwLock::new(Body {
                    kind: Kind::None,
                    job: None,
                    indexed_limit: 0,
                }),
                remaining_dependencies: AtomicU32::new(0),
                remaining_workers: AtomicU32::new(0),
            })
            .collect();
        let counters = (0..table_size * num_workers)
            .map(|_| Counter {
                index: AtomicU32::new(0),
                limit: AtomicU32::new(0),
            })
            .collect();
        Self {
            inner: Arc::new(Inner {
                num_workers,
                slots,
                counters,
                free: Mutex::new((0..free_slots as u32).collect()),
                free_cv: Condvar::new(),
                queue: ExecQueue::new(num_workers),
                idle: Mutex::new(0),
                idle_cv: Condvar::new(),
                stop: AtomicBool::new(false),
            }),
        }
    }

    /// `Tasks_NumWorkers`.
    pub fn num_workers(&self) -> usize {
        self.inner.num_workers
    }

    /// Starts the worker threads (`Task_Worker_<i>`). `on_start` runs on
    /// each worker before it takes any task -- the place for thread-locals
    /// and pinning. Call once.
    pub fn spawn_workers<F>(&self, on_start: F) -> Vec<thread::JoinHandle<()>>
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        let on_start = Arc::new(on_start);
        (0..self.inner.num_workers)
            .map(|w| {
                let inner = Arc::clone(&self.inner);
                let on_start = Arc::clone(&on_start);
                spawn_named(format!("Task_Worker_{w}"), move || {
                    on_start(w);
                    inner.worker_loop(w);
                })
            })
            .collect()
    }

    /// Makes every worker return from its loop once the queues are drained.
    /// The engine never calls this (workers live for the process, as in C);
    /// tests and loom models use it so threads can be joined.
    pub fn shutdown(&self) {
        let inner = &self.inner;
        inner.stop.store(true, Ordering::SeqCst);
        let _idle = lock(&inner.idle);
        inner.idle_cv.notify_all();
    }

    /// `Task_Allocate`: blocks until a slot is free, never fails.
    pub fn allocate(&self) -> TaskHandle {
        let inner = &self.inner;
        let index = {
            let mut free = lock(&inner.free);
            loop {
                if let Some(index) = free.pop_front() {
                    break index as usize;
                }
                free = inner.free_cv.wait(free).unwrap();
            }
        };
        let slot = &inner.slots[index];
        slot.remaining_dependencies.store(1, Ordering::Relaxed);
        {
            let mut body = slot.body.write().unwrap();
            body.kind = Kind::None;
            body.job = None;
            body.indexed_limit = 0;
        }
        let mut state = lock(&slot.state);
        state.dependents.clear();
        TaskHandle::new(index, state.epoch)
    }

    /// `Task_AssignFunc` (the payload copy is the caller's job).
    pub fn assign(&self, handle: TaskHandle, job: J) {
        let slot = &self.inner.slots[handle.index()];
        let mut body = slot.body.write().unwrap();
        body.kind = Kind::Scalar;
        body.job = Some(job);
    }

    /// `Task_AssignIndexedFunc`: `limit` indices chunked evenly over the
    /// workers (`count_per_worker = ceil(limit / num_workers)`).
    pub fn assign_indexed(&self, handle: TaskHandle, job: J, limit: u32) {
        let inner = &self.inner;
        let index = handle.index();
        let slot = &inner.slots[index];
        {
            let mut body = slot.body.write().unwrap();
            body.kind = Kind::Indexed;
            body.job = Some(job);
            body.indexed_limit = limit;
        }
        let num_workers = inner.num_workers as u32;
        let count_per_worker = limit.div_ceil(num_workers);
        let mut start = 0u32;
        for w in 0..inner.num_workers {
            let counter = &inner.counters[w * inner.slots.len() + index];
            counter.index.store(start, Ordering::Relaxed);
            counter
                .limit
                .store((start + count_per_worker).min(limit), Ordering::Relaxed);
            start += count_per_worker;
        }
    }

    /// `Task_Submit`: drops the implicit dependency and, if that was the
    /// last one, enqueues the task for `min(limit, num_workers)` workers
    /// (indexed) or one (scalar / unassigned).
    pub fn submit(&self, handle: TaskHandle) {
        self.inner.submit(handle);
    }

    /// `Task_AddDependency`: `after` waits for `before` unless `before` has
    /// already retired, in which case nothing happens (`after`'s count is
    /// left alone).
    pub fn add_dependency(&self, before: TaskHandle, after: TaskHandle) {
        let inner = &self.inner;
        let before_slot = &inner.slots[before.index()];
        let mut state = lock(&before_slot.state);
        if state.epoch != before.epoch() {
            return;
        }
        assert!(
            state.dependents.len() < MAX_DEPENDENT_TASKS,
            "Task_AddDependency: more than MAX_DEPENDENT_TASKS dependents"
        );
        state.dependents.push(after);
        inner.slots[after.index()]
            .remaining_dependencies
            .fetch_add(1, Ordering::AcqRel);
    }

    /// `Task_Join`: `true` once the slot's epoch has advanced past the
    /// handle's (immediately for a stale handle), `false` if `timeout`
    /// elapses first. A finite timeout restarts on every wake-up, as with
    /// `QCond_WaitTimeout`.
    pub fn join(&self, handle: TaskHandle, timeout: Timeout) -> bool {
        let slot = &self.inner.slots[handle.index()];
        let mut state = lock(&slot.state);
        while state.epoch == handle.epoch() {
            match timeout {
                Timeout::Infinite => state = slot.epoch_cv.wait(state).unwrap(),
                Timeout::Millis(ms) => {
                    let (guard, result) = slot
                        .epoch_cv
                        .wait_timeout(state, Duration::from_millis(u64::from(ms)))
                        .unwrap();
                    state = guard;
                    if result.timed_out() {
                        return false;
                    }
                }
            }
        }
        true
    }
}

impl<J: Job> Inner<J> {
    fn submit(&self, handle: TaskHandle) {
        let index = handle.index();
        let slot = &self.slots[index];
        debug_assert_eq!(
            lock(&slot.state).epoch,
            handle.epoch(),
            "Task_Submit: stale handle"
        );
        if slot.remaining_dependencies.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let num_task_workers = {
            let body = slot.body.read().unwrap();
            if body.kind == Kind::Indexed {
                body.indexed_limit.min(self.num_workers as u32)
            } else {
                1
            }
        };
        slot.remaining_workers
            .store(num_task_workers, Ordering::Release);
        for _ in 0..num_task_workers {
            self.queue.push(index as u32);
        }
        self.wake(num_task_workers as usize);
    }

    /// Wakes up to `n` idle workers. Taken under the idle lock so it cannot
    /// interleave with a worker between its empty re-check and its wait.
    fn wake(&self, n: usize) {
        let idle = lock(&self.idle);
        for _ in 0..n.min(*idle) {
            self.idle_cv.notify_one();
        }
    }

    /// `Task_Worker`'s loop.
    fn worker_loop(&self, w: usize) {
        let mut local = self.queue.take_local(w);
        while let Some(index) = self.next_task(&mut local, w) {
            self.execute(w, index as usize);
        }
    }

    /// Blocks until a task is available; `None` once `shutdown` was called
    /// and the queues are empty.
    fn next_task(&self, local: &mut <ExecQueue as Queue>::Local, w: usize) -> Option<u32> {
        loop {
            if let Some(index) = self.queue.pop(local, w) {
                return Some(index);
            }
            for _ in 0..WAIT_SPIN_COUNT {
                spin_hint();
                if let Some(index) = self.queue.pop(local, w) {
                    return Some(index);
                }
            }
            let mut idle = lock(&self.idle);
            if let Some(index) = self.queue.pop(local, w) {
                return Some(index);
            }
            if self.stop.load(Ordering::SeqCst) {
                return None;
            }
            *idle += 1;
            idle = self.idle_cv.wait(idle).unwrap();
            *idle -= 1;
        }
    }

    fn execute(&self, w: usize, index: usize) {
        let slot = &self.slots[index];
        {
            let body = slot.body.read().unwrap();
            match body.kind {
                Kind::None => {}
                Kind::Scalar => {
                    if let Some(job) = body.job.as_ref() {
                        job.run(None);
                    }
                }
                Kind::Indexed => {
                    if let Some(job) = body.job.as_ref() {
                        self.execute_indexed(w, index, job);
                    }
                }
            }
        }
        if slot.remaining_workers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.retire(index);
        }
    }

    /// `Task_ExecuteIndexed`: drain this worker's chunk, then the others'
    /// in `steal_worker_indices` order (`w`, `w + 1`, ... wrapping).
    fn execute_indexed(&self, w: usize, index: usize, job: &J) {
        for i in 0..self.num_workers {
            let steal_worker = (w + i) % self.num_workers;
            let counter = &self.counters[steal_worker * self.slots.len() + index];
            let limit = counter.limit.load(Ordering::Relaxed);
            loop {
                let i = counter.index.fetch_add(1, Ordering::AcqRel);
                if i >= limit {
                    break;
                }
                job.run(Some(i));
            }
        }
    }

    /// The last participating worker's exit path: submit the dependents,
    /// bump the epoch (waking joiners), give the slot back.
    fn retire(&self, index: usize) {
        let slot = &self.slots[index];
        {
            let mut state = lock(&slot.state);
            for i in 0..state.dependents.len() {
                let dependent = state.dependents[i];
                self.submit(dependent);
            }
            state.epoch += 1;
            slot.epoch_cv.notify_all();
        }
        let mut free = lock(&self.free);
        free.push_back(index as u32);
        self.free_cv.notify_one();
    }
}
