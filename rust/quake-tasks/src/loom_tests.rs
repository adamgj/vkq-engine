//! loom models of the scheduler state machine (ADR-016, plan D7). Built only
//! with `RUSTFLAGS=--cfg loom`; the queue is then the mutex-guarded
//! `VecDeque` (`queue::loom_impl`) and every std primitive is loom's.
//!
//! loom permits four threads in total, so each model uses at most three
//! workers, and every model shuts the scheduler down and joins its workers
//! so the exploration terminates.

use crate::{Job, Scheduler, Timeout};
use loom::model::Builder;
use loom::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use loom::sync::{Arc, Mutex};

struct Func<F>(F);

impl<F> Job for Func<F>
where
    F: Fn(Option<u32>) + Send + Sync + 'static,
{
    fn run(&self, index: Option<u32>) {
        (self.0)(index)
    }
}

type Sched = Scheduler<Func<Box<dyn Fn(Option<u32>) + Send + Sync + 'static>>>;

fn job<F>(f: F) -> Func<Box<dyn Fn(Option<u32>) + Send + Sync + 'static>>
where
    F: Fn(Option<u32>) + Send + Sync + 'static,
{
    Func(Box::new(f))
}

fn model(preemption_bound: usize, body: impl Fn() + Sync + Send + 'static) {
    let mut builder = Builder::new();
    builder.preemption_bound = Some(preemption_bound);
    builder.max_branches = 50_000;
    builder.check(body);
}

/// Spawns `workers` workers over a `table` slot table with `free` free
/// slots, runs `body`, then stops the workers.
fn with_scheduler(workers: usize, table: usize, free: usize, body: impl FnOnce(&Sched)) {
    let sched: Sched = Scheduler::with_table(workers, table, free);
    let handles = sched.spawn_workers(|_| {});
    body(&sched);
    sched.shutdown();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn chain_single_worker() {
    model(3, || {
        with_scheduler(1, 3, 2, |sched| {
            let order = Arc::new(Mutex::new(Vec::new()));
            let a = sched.allocate();
            let b = sched.allocate();
            let o = Arc::clone(&order);
            sched.assign(a, job(move |_| o.lock().unwrap().push(0)));
            let o = Arc::clone(&order);
            sched.assign(b, job(move |_| o.lock().unwrap().push(1)));
            sched.add_dependency(a, b);
            sched.submit(b);
            sched.submit(a);
            assert!(sched.join(b, Timeout::Infinite));
            assert_eq!(*order.lock().unwrap(), vec![0, 1]);
        });
    });
}

#[test]
fn diamond_two_workers() {
    model(2, || {
        with_scheduler(2, 5, 4, |sched| {
            let order = Arc::new(Mutex::new(Vec::new()));
            let mk = |tag: u32| {
                let o = Arc::clone(&order);
                let h = sched.allocate();
                sched.assign(h, job(move |_| o.lock().unwrap().push(tag)));
                h
            };
            let a = mk(0);
            let b = mk(1);
            let c = mk(2);
            let d = mk(3);
            sched.add_dependency(a, b);
            sched.add_dependency(a, c);
            sched.add_dependency(b, d);
            sched.add_dependency(c, d);
            for h in [d, c, b, a] {
                sched.submit(h);
            }
            assert!(sched.join(d, Timeout::Infinite));
            let order = order.lock().unwrap();
            assert_eq!(order.len(), 4);
            assert_eq!(order[0], 0);
            assert_eq!(order[3], 3);
        });
    });
}

#[test]
fn indexed_fan_out_two_workers() {
    model(2, || {
        with_scheduler(2, 2, 1, |sched| {
            let hits: Arc<Vec<AtomicU32>> = Arc::new((0..3).map(|_| AtomicU32::new(0)).collect());
            let h = sched.allocate();
            let hits2 = Arc::clone(&hits);
            sched.assign_indexed(
                h,
                job(move |i| {
                    hits2[i.unwrap() as usize].fetch_add(1, Ordering::Relaxed);
                }),
                3,
            );
            sched.submit(h);
            assert!(sched.join(h, Timeout::Infinite));
            for hit in hits.iter() {
                assert_eq!(hit.load(Ordering::Relaxed), 1);
            }
        });
    });
}

#[test]
fn join_timeout_races_completion() {
    model(3, || {
        with_scheduler(1, 2, 1, |sched| {
            let done = Arc::new(AtomicBool::new(false));
            let h = sched.allocate();
            let d = Arc::clone(&done);
            sched.assign(h, job(move |_| d.store(true, Ordering::SeqCst)));
            sched.submit(h);
            // loom decides whether the wait times out; `true` must imply the
            // job ran and a later infinite join must always succeed.
            if sched.join(h, Timeout::Millis(1)) {
                assert!(done.load(Ordering::SeqCst));
            }
            assert!(sched.join(h, Timeout::Infinite));
            assert!(done.load(Ordering::SeqCst));
            assert!(sched.join(h, Timeout::Millis(0)));
        });
    });
}

#[test]
fn epoch_reuse_is_not_aba() {
    model(2, || {
        with_scheduler(1, 3, 2, |sched| {
            let first = sched.allocate();
            let second = sched.allocate();
            sched.submit(first);
            sched.submit(second);
            // Blocks until one of the two retires and is recycled.
            let third = sched.allocate();
            assert!(third.index() == first.index() || third.index() == second.index());
            assert_eq!(third.epoch(), 1);
            if third.index() == first.index() {
                assert!(sched.join(first, Timeout::Millis(0)));
            } else {
                assert!(sched.join(second, Timeout::Millis(0)));
            }
            sched.submit(third);
            assert!(sched.join(third, Timeout::Infinite));
            assert!(sched.join(first, Timeout::Infinite));
            assert!(sched.join(second, Timeout::Infinite));
        });
    });
}
