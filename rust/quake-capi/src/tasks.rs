//! `tasks.c` -- the C ABI of the work-stealing job system (Rust migration
//! Phase 8 M2, ADR-016). The scheduler core is `quake-tasks`; this module
//! adds what C sees: the exported `Task_*`/`Tasks_*` names, the 128-byte
//! payload copy, the `-pinnedworkers` command line, worker pinning through
//! the C `Sys_PinCurrentThread`, and the thread-locals behind
//! `Tasks_IsWorker`/`Tasks_GetWorkerIndex`.
//!
//! Worker threads are plain `std::thread`s named `Task_Worker_<i>`; they are
//! never joined (the C workers are detached, too) and never host a
//! `setjmp` frame, so no `longjmp` can cross them (ADR-009).

use core::cell::{Cell, UnsafeCell};
use core::ffi::{c_int, c_void, CStr};
use core::ptr;
use quake_c_sys as c;
use quake_c_sys::tasks as g;
use quake_tasks::{Job, Scheduler, TaskHandle, Timeout, MAX_WORKERS};
use std::sync::{Arc, OnceLock};

/// `tasks.c` -- `MAX_PAYLOAD_SIZE`.
const MAX_PAYLOAD_SIZE: usize = 128;

/// `task_t`'s C half: the function pointer and the inline payload copy.
struct CJob {
    func: Option<unsafe extern "C" fn(*mut c_void)>,
    indexed: Option<unsafe extern "C" fn(c_int, *mut c_void)>,
    payload: UnsafeCell<[u8; MAX_PAYLOAD_SIZE]>,
}

// SAFETY: every worker running an indexed task receives the same mutable
// payload pointer, exactly as `tasks.c` hands out `task->payload`; whatever
// the task function does with those bytes is the C caller's contract, and
// the scheduler itself never reads them after the copy in `assign`.
unsafe impl Sync for CJob {}

impl Job for CJob {
    fn run(&self, index: Option<u32>) {
        let payload = self.payload.get().cast::<c_void>();
        match index {
            None => {
                if let Some(func) = self.func {
                    // SAFETY: `func` came from `Task_AssignFunc` and the payload
                    // is this task's own 128-byte buffer, alive for the call.
                    unsafe { func(payload) }
                }
            }
            Some(i) => {
                if let Some(func) = self.indexed {
                    // SAFETY: as above, from `Task_AssignIndexedFunc`; `i` is
                    // below the caller's `limit`, which C passes as `int`.
                    unsafe { func(i as c_int, payload) }
                }
            }
        }
    }
}

struct State {
    sched: Scheduler<CJob>,
    num_workers: c_int,
}

static STATE: OnceLock<State> = OnceLock::new();

thread_local! {
    /// `tasks.c` -- `is_worker`.
    static IS_WORKER: Cell<bool> = const { Cell::new(false) };
    /// `tasks.c` -- `tl_worker_index` (0 on the main thread).
    static WORKER_INDEX: Cell<c_int> = const { Cell::new(0) };
}

fn sched() -> &'static Scheduler<CJob> {
    &STATE.get().expect("Task_* called before Tasks_Init").sched
}

fn new_job(payload: *mut c_void, payload_size: usize) -> CJob {
    assert!(
        payload_size <= MAX_PAYLOAD_SIZE,
        "task payload exceeds MAX_PAYLOAD_SIZE"
    );
    let job = CJob {
        func: None,
        indexed: None,
        payload: UnsafeCell::new([0; MAX_PAYLOAD_SIZE]),
    };
    if !payload.is_null() {
        // SAFETY: the caller passes `payload_size` readable bytes at
        // `payload` (the `tasks.h` contract, `memcpy` in C); the destination
        // is this job's own buffer, checked to hold `payload_size` above.
        unsafe {
            ptr::copy_nonoverlapping(
                payload.cast::<u8>(),
                job.payload.get().cast::<u8>(),
                payload_size,
            );
        }
    }
    job
}

/// `tasks.c` -- `parse_pinned_workers`'s command-line lookup: the argument
/// after `-pinnedworkers`, if present.
unsafe fn pinned_workers_arg() -> Option<Vec<u8>> {
    // SAFETY: `COM_InitArgv` has run (Tasks_Init is called from Host_Init
    // after it), so `com_argc`/`com_argv` are populated and every entry
    // below `com_argc` is a NUL-terminated string.
    unsafe {
        let index = c::COM_CheckParm(c"-pinnedworkers".as_ptr());
        let argc = ptr::addr_of!(c::com_argc).read();
        if index == 0 || index >= argc - 1 {
            return None;
        }
        let argv = ptr::addr_of!(c::com_argv).read();
        let arg = *argv.add(index as usize + 1);
        if arg.is_null() {
            return None;
        }
        Some(CStr::from_ptr(arg).to_bytes().to_vec())
    }
}

/// `tasks.c` -- `Tasks_Init`. Sizes the pool (`CLAMP (1, cores, 32)`,
/// overridden by a valid `-pinnedworkers` list), then starts the workers.
///
/// # Safety
///
/// Call once, from the main thread, after `COM_InitArgv`.
#[no_mangle]
pub unsafe extern "C" fn Tasks_Init() {
    // SAFETY: plain C callee with no preconditions.
    let cores = unsafe { g::QThread_NumLogicalCores() };
    let mut num_workers = cores.clamp(1, MAX_WORKERS as c_int) as usize;
    // SAFETY: the caller guarantees the argv state (see the function doc).
    let pinned = unsafe { pinned_workers_arg() }
        .map(|arg| quake_tasks::pinned::parse(&arg, cores.max(1) as usize))
        .unwrap_or_default();
    if !pinned.is_empty() {
        num_workers = pinned.len();
    }

    let sched = Scheduler::new(num_workers);
    let pinned = Arc::new(pinned);
    // The join handles are dropped: the workers live for the process, like
    // the detached `QThread_Create` workers in C.
    let _detached = sched.spawn_workers(move |worker_index| {
        IS_WORKER.set(true);
        WORKER_INDEX.set(worker_index as c_int);
        if let Some(&core) = pinned.get(worker_index) {
            // SAFETY: plain C callees; `Sys_PinCurrentThread` acts on the
            // calling thread, and `Con_DPrintf` takes the C console lock
            // itself (it is what the C worker calls from this very spot).
            unsafe {
                if !g::Sys_PinCurrentThread(core as c_int) {
                    c::Con_DPrintf(
                        c"Tasks : Failed to pin worker %d (N/A or no access rights)".as_ptr(),
                        worker_index as c_int,
                    );
                }
            }
        }
    });

    assert!(
        STATE
            .set(State {
                sched,
                num_workers: num_workers as c_int,
            })
            .is_ok(),
        "Tasks_Init called twice"
    );
}

/// `tasks.c` -- `Tasks_NumWorkers` (0 before `Tasks_Init`, as in C).
#[no_mangle]
pub extern "C" fn Tasks_NumWorkers() -> c_int {
    STATE.get().map_or(0, |s| s.num_workers)
}

/// `tasks.c` -- `Tasks_IsWorker`.
#[no_mangle]
pub extern "C" fn Tasks_IsWorker() -> bool {
    IS_WORKER.get()
}

/// `tasks.c` -- `Tasks_GetWorkerIndex`: the worker's index, 0 on any other
/// thread; always below `TASKS_MAX_WORKERS`.
#[no_mangle]
pub extern "C" fn Tasks_GetWorkerIndex() -> c_int {
    WORKER_INDEX.get()
}

/// `tasks.c` -- `Task_Allocate`. Blocks until a slot is free.
#[no_mangle]
pub extern "C" fn Task_Allocate() -> u64 {
    sched().allocate().raw()
}

/// `tasks.c` -- `Task_AssignFunc`.
///
/// # Safety
///
/// `handle` is live (allocated, not yet submitted); `payload` is null or
/// points at `payload_size` readable bytes, `payload_size <= 128`.
#[no_mangle]
pub unsafe extern "C" fn Task_AssignFunc(
    handle: u64,
    func: Option<unsafe extern "C" fn(*mut c_void)>,
    payload: *mut c_void,
    payload_size: usize,
) {
    let mut job = new_job(payload, payload_size);
    job.func = func;
    sched().assign(TaskHandle::from_raw(handle), job);
}

/// `tasks.c` -- `Task_AssignIndexedFunc`.
///
/// # Safety
///
/// As [`Task_AssignFunc`].
#[no_mangle]
pub unsafe extern "C" fn Task_AssignIndexedFunc(
    handle: u64,
    func: Option<unsafe extern "C" fn(c_int, *mut c_void)>,
    limit: u32,
    payload: *mut c_void,
    payload_size: usize,
) {
    let mut job = new_job(payload, payload_size);
    job.indexed = func;
    sched().assign_indexed(TaskHandle::from_raw(handle), job, limit);
}

/// `tasks.c` -- `Task_Submit`.
#[no_mangle]
pub extern "C" fn Task_Submit(handle: u64) {
    sched().submit(TaskHandle::from_raw(handle));
}

/// `tasks.c` -- `Tasks_Submit`: `Task_Submit` over an array.
///
/// # Safety
///
/// `handles` points at `num_handles` readable handles.
#[no_mangle]
pub unsafe extern "C" fn Tasks_Submit(num_handles: c_int, handles: *mut u64) {
    let sched = sched();
    for i in 0..num_handles.max(0) as usize {
        // SAFETY: `i < num_handles`, within the caller's array.
        let handle = unsafe { *handles.add(i) };
        sched.submit(TaskHandle::from_raw(handle));
    }
}

/// `tasks.c` -- `Task_AddDependency`: a no-op once `before` has retired.
#[no_mangle]
pub extern "C" fn Task_AddDependency(before: u64, after: u64) {
    sched().add_dependency(TaskHandle::from_raw(before), TaskHandle::from_raw(after));
}

/// `tasks.c` -- `Task_Join`. `TASK_TIMEOUT_INFINITE` is `0xFFFFFFFF` under
/// both SDL2 (`SDL_MUTEX_MAXWAIT`) and SDL3 (`(uint32_t) - 1`), so the
/// sentinel is fixed here rather than read from the header.
#[no_mangle]
pub extern "C" fn Task_Join(handle: u64, timeout: u32) -> bool {
    let timeout = if timeout == u32::MAX {
        Timeout::Infinite
    } else {
        Timeout::Millis(timeout)
    };
    sched().join(TaskHandle::from_raw(handle), timeout)
}
