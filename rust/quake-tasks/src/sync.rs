//! std / loom switch for every primitive the scheduler uses.
//!
//! The loom side is only reachable from this crate's own unit-test target
//! (`cargo test -p quake-tasks` with `RUSTFLAGS=--cfg loom`): loom is a
//! dev-dependency, so a plain `--cfg loom` library build still uses std.

#[cfg(all(test, loom))]
pub(crate) use loom::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Condvar, Mutex, MutexGuard, RwLock,
    },
    thread,
};

#[cfg(not(all(test, loom)))]
pub(crate) use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Condvar, Mutex, MutexGuard, RwLock,
    },
    thread,
};

/// Locks `m`. Poisoning cannot happen with the workspace's `panic = "abort"`
/// profiles; the unwrap documents that rather than hiding it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap()
}

/// One scheduling point inside a spin loop. Under loom every yield is a
/// branch the model explores, so the caller bounds the spin.
#[cfg(all(test, loom))]
#[allow(dead_code)] // WAIT_SPIN_COUNT is 0 under loom
pub(crate) fn spin_hint() {
    loom::thread::yield_now();
}

#[cfg(not(all(test, loom)))]
pub(crate) fn spin_hint() {
    core::hint::spin_loop();
}

/// Spawns a worker thread named `name`. loom has no `Builder`, so the name
/// is dropped there.
#[cfg(all(test, loom))]
pub(crate) fn spawn_named<F>(_name: String, f: F) -> thread::JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    thread::spawn(f)
}

#[cfg(not(all(test, loom)))]
pub(crate) fn spawn_named<F>(name: String, f: F) -> thread::JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    thread::Builder::new()
        .name(name)
        .spawn(f)
        .expect("Tasks_Init: failed to spawn worker thread")
}
