//! Behavioural tests over the observable contract (R1); the `-pinnedworkers`
//! parser has its own tests in `pinned.rs`.

use crate::{Job, Scheduler, TaskHandle, Timeout, MAX_PENDING_TASKS};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A closure job.
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

/// Runs `body` against a scheduler with `workers` workers and `free` free
/// slots, then stops and joins the workers.
fn with_scheduler(workers: usize, free: usize, body: impl FnOnce(&Sched)) {
    let sched: Sched = Scheduler::with_free_slots(workers, free);
    let handles = sched.spawn_workers(|_| {});
    body(&sched);
    sched.shutdown();
    for h in handles {
        h.join().unwrap();
    }
}

/// A one-shot gate a task can block on.
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(false),
            cv: Condvar::new(),
        })
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.cv.wait(open).unwrap();
        }
    }

    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }
}

#[test]
fn handle_encoding_matches_tasks_c() {
    let h = TaskHandle::from_raw(5 | (3 << 8));
    assert_eq!(h.index(), 5);
    assert_eq!(h.epoch(), 3);
    assert_eq!(h.raw(), 5 | (3 << 8));
    let sched: Sched = Scheduler::new(1);
    let first = sched.allocate();
    assert_eq!(first.epoch(), 0);
    assert!(first.index() < MAX_PENDING_TASKS);
    assert_ne!(first.raw(), crate::INVALID_TASK_HANDLE);
}

#[test]
fn free_ring_is_seeded_with_255_slots() {
    let sched: Sched = Scheduler::new(1);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..MAX_PENDING_TASKS - 1 {
        assert!(seen.insert(sched.allocate().index()));
    }
    assert_eq!(seen.len(), 255);
}

#[test]
fn single_worker_runs_independent_tasks_in_submission_order() {
    with_scheduler(1, 200, |sched| {
        let order = Arc::new(Mutex::new(Vec::new()));
        let gate = Gate::new();
        let mut handles = Vec::new();
        for i in 0..100u32 {
            let order = Arc::clone(&order);
            let gate = Arc::clone(&gate);
            let h = sched.allocate();
            sched.assign(
                h,
                job(move |_| {
                    gate.wait();
                    order.lock().unwrap().push(i);
                }),
            );
            sched.submit(h);
            handles.push(h);
        }
        gate.open();
        for h in handles {
            assert!(sched.join(h, Timeout::Infinite));
        }
        assert_eq!(*order.lock().unwrap(), (0..100).collect::<Vec<_>>());
    });
}

#[test]
fn dependency_chain_orders_execution() {
    for workers in [1, 4] {
        with_scheduler(workers, 254, |sched| {
            let order = Arc::new(Mutex::new(Vec::new()));
            let handles: Vec<_> = (0..16u32)
                .map(|i| {
                    let order = Arc::clone(&order);
                    let h = sched.allocate();
                    sched.assign(h, job(move |_| order.lock().unwrap().push(i)));
                    h
                })
                .collect();
            for w in handles.windows(2) {
                sched.add_dependency(w[0], w[1]);
            }
            // Submit in reverse so ordering can only come from the edges.
            for &h in handles.iter().rev() {
                sched.submit(h);
            }
            assert!(sched.join(*handles.last().unwrap(), Timeout::Infinite));
            assert_eq!(*order.lock().unwrap(), (0..16).collect::<Vec<_>>());
        });
    }
}

#[test]
fn diamond_runs_sink_after_both_branches() {
    with_scheduler(3, 254, |sched| {
        for _ in 0..200 {
            let order = Arc::new(Mutex::new(Vec::new()));
            let mk = |tag: &'static str| {
                let order = Arc::clone(&order);
                let h = sched.allocate();
                sched.assign(h, job(move |_| order.lock().unwrap().push(tag)));
                h
            };
            let a = mk("a");
            let b = mk("b");
            let c = mk("c");
            let d = mk("d");
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
            assert_eq!(order[0], "a");
            assert_eq!(order[3], "d");
        }
    });
}

#[test]
fn indexed_task_covers_every_index_exactly_once() {
    for (workers, limit) in [(1, 1000u32), (3, 1000), (3, 2), (3, 3), (8, 7), (4, 0)] {
        with_scheduler(workers, 254, |sched| {
            let hits: Arc<Vec<AtomicU32>> =
                Arc::new((0..limit).map(|_| AtomicU32::new(0)).collect());
            let h = sched.allocate();
            let hits2 = Arc::clone(&hits);
            sched.assign_indexed(
                h,
                job(move |i| {
                    hits2[i.unwrap() as usize].fetch_add(1, Ordering::Relaxed);
                }),
                limit,
            );
            sched.submit(h);
            if limit == 0 {
                // C never retires a zero-limit indexed task (remaining_workers
                // starts at 0); the handle stays pending.
                assert!(!sched.join(h, Timeout::Millis(50)));
            } else {
                assert!(sched.join(h, Timeout::Infinite));
                for (i, hit) in hits.iter().enumerate() {
                    assert_eq!(hit.load(Ordering::Relaxed), 1, "index {i}");
                }
            }
        });
    }
}

#[test]
fn indexed_task_runs_index_zero_for_limit_one() {
    with_scheduler(4, 254, |sched| {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let h = sched.allocate();
        let seen2 = Arc::clone(&seen);
        sched.assign_indexed(h, job(move |i| seen2.lock().unwrap().push(i)), 1);
        sched.submit(h);
        assert!(sched.join(h, Timeout::Infinite));
        assert_eq!(*seen.lock().unwrap(), vec![Some(0)]);
    });
}

#[test]
fn join_timeout_is_bounded_then_infinite_then_stale() {
    with_scheduler(1, 254, |sched| {
        let gate = Gate::new();
        let done = Arc::new(AtomicBool::new(false));
        let h = sched.allocate();
        let (gate2, done2) = (Arc::clone(&gate), Arc::clone(&done));
        sched.assign(
            h,
            job(move |_| {
                gate2.wait();
                done2.store(true, Ordering::SeqCst);
            }),
        );
        sched.submit(h);

        let start = Instant::now();
        assert!(!sched.join(h, Timeout::Millis(20)));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(15), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        assert!(!done.load(Ordering::SeqCst));
        assert!(!sched.join(h, Timeout::Millis(0)));

        gate.open();
        assert!(sched.join(h, Timeout::Infinite));
        assert!(done.load(Ordering::SeqCst));
        assert!(sched.join(h, Timeout::Infinite));
        assert!(sched.join(h, Timeout::Millis(0)));
    });
}

#[test]
fn unassigned_task_retires() {
    with_scheduler(2, 254, |sched| {
        let h = sched.allocate();
        sched.submit(h);
        assert!(sched.join(h, Timeout::Infinite));
    });
}

#[test]
fn add_dependency_on_retired_before_is_a_no_op() {
    with_scheduler(2, 254, |sched| {
        let a = sched.allocate();
        sched.submit(a);
        assert!(sched.join(a, Timeout::Infinite));

        let ran = Arc::new(AtomicBool::new(false));
        let b = sched.allocate();
        let ran2 = Arc::clone(&ran);
        sched.assign(b, job(move |_| ran2.store(true, Ordering::SeqCst)));
        sched.add_dependency(a, b);
        sched.submit(b);
        assert!(sched.join(b, Timeout::Infinite));
        assert!(ran.load(Ordering::SeqCst));
    });
}

#[test]
fn dependent_submitted_before_its_dependency_waits() {
    with_scheduler(2, 254, |sched| {
        let gate = Gate::new();
        let a_done = Arc::new(AtomicBool::new(false));
        let a = sched.allocate();
        let (gate2, a_done2) = (Arc::clone(&gate), Arc::clone(&a_done));
        sched.assign(
            a,
            job(move |_| {
                gate2.wait();
                a_done2.store(true, Ordering::SeqCst);
            }),
        );
        let b = sched.allocate();
        let a_done3 = Arc::clone(&a_done);
        let b_saw_a = Arc::new(AtomicBool::new(false));
        let b_saw_a2 = Arc::clone(&b_saw_a);
        sched.assign(
            b,
            job(move |_| b_saw_a2.store(a_done3.load(Ordering::SeqCst), Ordering::SeqCst)),
        );
        sched.add_dependency(a, b);
        sched.submit(b);
        assert!(!sched.join(b, Timeout::Millis(20)));
        sched.submit(a);
        assert!(!sched.join(b, Timeout::Millis(20)));
        gate.open();
        assert!(sched.join(b, Timeout::Infinite));
        assert!(b_saw_a.load(Ordering::SeqCst));
    });
}

#[test]
fn lots_of_tasks() {
    with_scheduler(4, 255, |sched| {
        let count = Arc::new(AtomicUsize::new(0));
        let mut pending = Vec::new();
        for _ in 0..100_000 {
            let h = sched.allocate();
            let count = Arc::clone(&count);
            sched.assign(
                h,
                job(move |_| {
                    count.fetch_add(1, Ordering::Relaxed);
                }),
            );
            sched.submit(h);
            pending.push(h);
            if pending.len() == 128 {
                for h in pending.drain(..) {
                    assert!(sched.join(h, Timeout::Infinite));
                }
            }
        }
        for h in pending {
            assert!(sched.join(h, Timeout::Infinite));
        }
        assert_eq!(count.load(Ordering::Relaxed), 100_000);
    });
}

#[test]
fn allocate_blocks_until_a_slot_retires() {
    with_scheduler(1, 2, |sched| {
        let gate = Gate::new();
        let mut held = Vec::new();
        for _ in 0..2 {
            let h = sched.allocate();
            let gate = Arc::clone(&gate);
            sched.assign(h, job(move |_| gate.wait()));
            sched.submit(h);
            held.push(h);
        }
        let sched2 = sched.clone();
        let got = Arc::new(AtomicBool::new(false));
        let got2 = Arc::clone(&got);
        let allocator = thread::spawn(move || {
            let h = sched2.allocate();
            got2.store(true, Ordering::SeqCst);
            h
        });
        thread::sleep(Duration::from_millis(50));
        assert!(!got.load(Ordering::SeqCst));
        gate.open();
        let third = allocator.join().unwrap();
        assert!(got.load(Ordering::SeqCst));
        for h in held {
            assert!(sched.join(h, Timeout::Infinite));
        }
        sched.submit(third);
        assert!(sched.join(third, Timeout::Infinite));
    });
}

#[test]
fn slot_reuse_advances_the_epoch() {
    with_scheduler(1, 1, |sched| {
        let first = sched.allocate();
        sched.submit(first);
        assert!(sched.join(first, Timeout::Infinite));
        let second = sched.allocate();
        assert_eq!(second.index(), first.index());
        assert_eq!(second.epoch(), first.epoch() + 1);
        assert!(sched.join(first, Timeout::Millis(0)));
        assert!(!sched.join(second, Timeout::Millis(0)));
        sched.submit(second);
        assert!(sched.join(second, Timeout::Infinite));
        assert_eq!(sched.allocate().epoch(), first.epoch() + 2);
    });
}

#[test]
fn worker_start_hook_runs_on_each_worker_first() {
    let sched: Sched = Scheduler::new(3);
    let started = Arc::new(Mutex::new(Vec::new()));
    let names = Arc::new(Mutex::new(Vec::new()));
    let (s2, n2) = (Arc::clone(&started), Arc::clone(&names));
    let handles = sched.spawn_workers(move |w| {
        s2.lock().unwrap().push(w);
        n2.lock()
            .unwrap()
            .push(thread::current().name().unwrap().to_owned());
    });
    let h = sched.allocate();
    sched.submit(h);
    assert!(sched.join(h, Timeout::Infinite));
    sched.shutdown();
    for h in handles {
        h.join().unwrap();
    }
    let mut started = started.lock().unwrap().clone();
    started.sort_unstable();
    assert_eq!(started, vec![0, 1, 2]);
    let mut names = names.lock().unwrap().clone();
    names.sort();
    assert_eq!(
        names,
        vec!["Task_Worker_0", "Task_Worker_1", "Task_Worker_2"]
    );
}

#[test]
fn num_workers_is_reported() {
    let sched: Sched = Scheduler::new(7);
    assert_eq!(sched.num_workers(), 7);
}
