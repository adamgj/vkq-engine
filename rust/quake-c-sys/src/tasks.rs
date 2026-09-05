//! Hand-written externs for the task system (Rust migration Phase 8 M2,
//! ADR-016). `tasks.h`, `sys.h`'s pinning entry and `q_thread.h` are not
//! bindgen roots (`bindings_wrapper.h`), so the C callees the Rust scheduler
//! shim needs, and the `Tasks_*` queries the ported host/particle code asks,
//! are declared here once. The three queries resolve to `tasks.c` in a
//! C-tasks build and to `quake-capi`'s exports under `-Duse_rust_tasks`;
//! the ABI is the same either way.

use core::ffi::c_int;

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
}
