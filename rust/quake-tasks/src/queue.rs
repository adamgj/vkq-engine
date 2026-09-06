//! The executable-task queue behind a small trait (plan D3 / RA3): the
//! crossbeam deque family for real builds, a mutex-guarded `VecDeque` for the
//! loom models, which cannot see inside crossbeam's lock-free code.
//!
//! Both are FIFO from the point of view of a single consumer, which is what
//! keeps single-worker execution order equal to C's global ring: every
//! submission goes to the shared injector, and a worker's local deque only
//! ever holds a batch moved out of the injector in order.

/// A multi-producer queue of slot indices with one owner-side handle per
/// worker.
pub(crate) trait Queue: Send + Sync {
    /// Per-worker state; handed to the worker thread once.
    type Local: Send;

    fn new(num_workers: usize) -> Self;
    /// Takes worker `w`'s handle. Panics if taken twice.
    fn take_local(&self, w: usize) -> Self::Local;
    /// Enqueues from any thread.
    fn push(&self, index: u32);
    /// Dequeues for the worker owning `local`, stealing from the others when
    /// its own share is empty. `None` means every queue looked empty.
    fn pop(&self, local: &mut Self::Local, w: usize) -> Option<u32>;
}

#[cfg(not(all(test, loom)))]
pub(crate) use crossbeam_impl::CrossbeamQueue as ExecQueue;
#[cfg(all(test, loom))]
pub(crate) use loom_impl::MutexQueue as ExecQueue;

#[cfg(not(all(test, loom)))]
mod crossbeam_impl {
    use super::Queue;
    use crate::sync::{lock, Mutex};
    use crossbeam_deque::{Injector, Steal, Stealer, Worker};

    pub(crate) struct CrossbeamQueue {
        injector: Injector<u32>,
        stealers: Vec<Stealer<u32>>,
        locals: Mutex<Vec<Option<Worker<u32>>>>,
    }

    impl Queue for CrossbeamQueue {
        type Local = Worker<u32>;

        fn new(num_workers: usize) -> Self {
            let workers: Vec<Worker<u32>> = (0..num_workers).map(|_| Worker::new_fifo()).collect();
            let stealers = workers.iter().map(Worker::stealer).collect();
            Self {
                injector: Injector::new(),
                stealers,
                locals: Mutex::new(workers.into_iter().map(Some).collect()),
            }
        }

        fn take_local(&self, w: usize) -> Worker<u32> {
            lock(&self.locals)[w]
                .take()
                .expect("worker deque already taken")
        }

        fn push(&self, index: u32) {
            self.injector.push(index);
        }

        fn pop(&self, local: &mut Worker<u32>, w: usize) -> Option<u32> {
            if let Some(index) = local.pop() {
                return Some(index);
            }
            loop {
                match self.injector.steal_batch_and_pop(local) {
                    Steal::Success(index) => return Some(index),
                    Steal::Empty => break,
                    Steal::Retry => continue,
                }
            }
            let n = self.stealers.len();
            for i in 1..n {
                let victim = &self.stealers[(w + i) % n];
                loop {
                    match victim.steal() {
                        Steal::Success(index) => return Some(index),
                        Steal::Empty => break,
                        Steal::Retry => continue,
                    }
                }
            }
            None
        }
    }
}

#[cfg(all(test, loom))]
mod loom_impl {
    use super::Queue;
    use crate::sync::{lock, Mutex};
    use std::collections::VecDeque;

    pub(crate) struct MutexQueue {
        items: Mutex<VecDeque<u32>>,
    }

    impl Queue for MutexQueue {
        type Local = ();

        fn new(_num_workers: usize) -> Self {
            Self {
                items: Mutex::new(VecDeque::new()),
            }
        }

        fn take_local(&self, _w: usize) {}

        fn push(&self, index: u32) {
            lock(&self.items).push_back(index);
        }

        fn pop(&self, _local: &mut (), _w: usize) -> Option<u32> {
            lock(&self.items).pop_front()
        }
    }
}
