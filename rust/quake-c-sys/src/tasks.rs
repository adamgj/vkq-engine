//! Hand-written externs for the task system (Rust migration Phase 8 M2,
//! ADR-016). `tasks.h`, `sys.h`'s pinning entry and `q_thread.h` are not
//! bindgen roots (`bindings_wrapper.h`), so the C callees the Rust scheduler
//! shim needs, and the `Tasks_*` queries the ported host/particle code asks,
//! are declared here once. The three queries resolve to `tasks.c` in a
//! C-tasks build and to `quake-capi`'s exports under `-Duse_rust_tasks`;
//! the ABI is the same either way.

use core::ffi::{c_int, c_void};

extern "C" {
    /// `sys.h:165` -- `bool Sys_PinCurrentThread (int core_index)`. Returns
    /// `false` where pinning is unsupported (macOS/BSD) or denied.
    pub fn Sys_PinCurrentThread(core_index: c_int) -> bool;
    /// `q_thread.h:64` -- `int QThread_NumLogicalCores (void)`.
    pub fn QThread_NumLogicalCores() -> c_int;

    /// `tasks.h` -- `qboolean Tasks_IsWorker (void)` (`qboolean` is `bool`).
    pub fn Tasks_IsWorker() -> bool;
    /// `tasks.h` -- `int Tasks_GetWorkerIndex (void)`.
    pub fn Tasks_GetWorkerIndex() -> c_int;
    /// `tasks.h` -- `int Tasks_NumWorkers (void)`.
    pub fn Tasks_NumWorkers() -> c_int;

    /* Phase 8 M6: the frame-task entry points gl_vidsdl.c uses. `use_rust_
     * render` and `use_rust_tasks` are orthogonal, so the ported renderer
     * goes through the C ABI rather than `quake-tasks` directly. */

    /// `tasks.h` -- `task_handle_t Task_Allocate (void)`.
    pub fn Task_Allocate() -> u64;
    /// `tasks.h` -- `void Task_AssignFunc (task_handle_t task_handle,
    /// task_func_t func, void *payload, size_t payload_size)`.
    pub fn Task_AssignFunc(
        task_handle: u64,
        func: Option<unsafe extern "C" fn(*mut c_void)>,
        payload: *mut c_void,
        payload_size: usize,
    );
    /// `tasks.h` -- `qboolean Task_Join (task_handle_t task_handle, uint32_t
    /// timeout)`; `TASK_TIMEOUT_INFINITE` is `UINT32_MAX` under SDL2 and
    /// SDL3 alike.
    pub fn Task_Join(task_handle: u64, timeout: u32) -> bool;
    /// `tasks.h:42` -- `void Task_AssignIndexedFunc (task_handle_t, task_indexed_func_t,
    /// uint32_t limit, void *payload, size_t payload_size)`.
    pub fn Task_AssignIndexedFunc(
        task_handle: u64,
        func: Option<unsafe extern "C" fn(c_int, *mut c_void)>,
        limit: u32,
        payload: *mut c_void,
        payload_size: usize,
    );
    /// `tasks.h:43` -- `void Task_Submit (task_handle_t)`.
    pub fn Task_Submit(task_handle: u64);
    /// `tasks.h:44` -- `void Tasks_Submit (int num_handles, task_handle_t *handles)`.
    pub fn Tasks_Submit(num_handles: c_int, handles: *mut u64);
    /// `tasks.h:45` -- `void Task_AddDependency (task_handle_t before, task_handle_t after)`.
    pub fn Task_AddDependency(before: u64, after: u64);
}
