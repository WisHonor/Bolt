//! Worker threads for one command. They read, hash, check and compress; the thread that starts
//! them takes their results in the jobs' order and is the only one that writes packs, so a command
//! writes the same packs, and stops at the same first problem, whatever the timing.

use std::collections::VecDeque;
use std::iter::Fuse;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, Scope};

/// The most workers, however many threads the machine runs at once.
const MOST_WORKERS: usize = 8;
/// The most jobs out at once, for each worker.
const JOBS_PER_WORKER: usize = 4;
/// About the most the jobs out at once may hold.
pub(crate) const WINDOW: usize = 64 << 20;

/// The workers of one command: how many, and the test hooks each one starts with.
pub(crate) struct Workers {
    count: usize,
    hooks: Hooks,
}

/// Runs `f` with the workers of one command: as many as the machine runs at once, at most 8.
/// Each call that hands them jobs starts the threads it needs and ends them before it returns.
pub(crate) fn with_workers<R>(f: impl FnOnce(&Workers) -> R) -> R {
    f(&Workers {
        count: count(),
        hooks: Hooks::here(),
    })
}

fn count() -> usize {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(count) = TEST_COUNT.get() {
        return count.clamp(1, MOST_WORKERS);
    }
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(|| {
        thread::available_parallelism()
            .map_or(1, usize::from)
            .clamp(1, MOST_WORKERS)
    })
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_COUNT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Makes the commands this thread runs use `count` workers, so a test can compare one with
/// eight; `None` goes back to the machine's own.
#[cfg(any(test, feature = "test-support"))]
pub fn set_test_workers(count: Option<usize>) {
    TEST_COUNT.set(count);
}

impl Workers {
    /// The most jobs out at once.
    pub(crate) fn most_out(&self) -> usize {
        self.count.saturating_mul(JOBS_PER_WORKER)
    }

    /// Runs `work` on each job on the workers and hands the results to `take` in the jobs' order,
    /// stopping at the first error it gives. A job counts `cost(&job)` (what it reads plus the
    /// most its result can be) from when it is handed out until its result is taken; none is
    /// handed out while that total would pass `window`, or 4 jobs a worker are out.
    pub(crate) fn in_order<J: Send, T: Send, E>(
        &self,
        jobs: impl IntoIterator<Item = J>,
        window: usize,
        cost: impl Fn(&J) -> usize,
        work: impl Fn(J) -> T + Sync,
        mut take: impl FnMut(T) -> Result<(), E>,
    ) -> Result<(), E> {
        self.results(jobs, window, cost, work, |results| {
            for result in results {
                take(result)?;
            }
            Ok(())
        })
    }

    /// `in_order`, with `with` asking for each result when it needs it, so the thread that takes
    /// them can do its own work in between. Jobs are handed out only when it asks.
    pub(crate) fn results<J: Send, T: Send, R>(
        &self,
        jobs: impl IntoIterator<Item = J>,
        window: usize,
        cost: impl Fn(&J) -> usize,
        work: impl Fn(J) -> T + Sync,
        with: impl FnOnce(&mut dyn Iterator<Item = T>) -> R,
    ) -> R {
        let shared = Shared {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                slots: VecDeque::new(),
                first: 0,
                closed: false,
                lost: false,
            }),
            job: Condvar::new(),
            result: Condvar::new(),
        };
        thread::scope(|scope| {
            let mut out = Out {
                scope,
                shared: &shared,
                work: &work,
                hooks: &self.hooks,
                jobs: jobs.into_iter().fuse(),
                cost,
                window,
                count: self.count,
                most: self.most_out(),
                waiting: None,
                costs: VecDeque::new(),
                held: 0,
                started: 0,
            };
            // `out` closes the queue when it goes, so the workers end before the scope waits.
            with(&mut out)
        })
    }
}

struct Shared<J, T> {
    state: Mutex<State<J, T>>,
    /// Wakes the workers: a job was handed out, or there will be no more.
    job: Condvar,
    /// Wakes the thread taking results: one came in, or a worker stopped part way.
    result: Condvar,
}

struct State<J, T> {
    /// Jobs handed out and not started yet, each with its place in the order.
    queue: VecDeque<(usize, J)>,
    /// A slot for each job out, from the one whose result is taken next.
    slots: VecDeque<Option<T>>,
    /// The place in the order of the first slot.
    first: usize,
    /// No more jobs will come.
    closed: bool,
    /// A worker stopped without giving its result.
    lost: bool,
}

impl<J, T> Shared<J, T> {
    fn lock(&self) -> MutexGuard<'_, State<J, T>> {
        // Nothing is left half done under the lock, whoever held it.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next job a worker should do, or `None` once there are no more.
    fn next_job(&self) -> Option<(usize, J)> {
        let mut state = self.lock();
        loop {
            if let Some(job) = state.queue.pop_front() {
                return Some(job);
            }
            if state.closed {
                return None;
            }
            state = self.job.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Never waits: the result goes in its slot, whatever the thread taking results is doing.
    fn put(&self, at: usize, result: T) {
        self.lock().put(at, result);
        self.result.notify_one();
    }
}

impl<J, T> State<J, T> {
    fn put(&mut self, at: usize, result: T) {
        // Gone once the results stopped being taken.
        let slot = at
            .checked_sub(self.first)
            .and_then(|i| self.slots.get_mut(i));
        if let Some(slot) = slot {
            *slot = Some(result);
        }
    }
}

/// The thread that takes the results: it hands out jobs as the window allows, and waits for each
/// result in turn.
struct Out<'scope, 'env, J, T, I, C, W>
where
    I: Iterator<Item = J>,
    C: Fn(&J) -> usize,
    W: Fn(J) -> T + Sync,
{
    scope: &'scope Scope<'scope, 'env>,
    shared: &'scope Shared<J, T>,
    work: &'scope W,
    hooks: &'scope Hooks,
    jobs: Fuse<I>,
    cost: C,
    window: usize,
    count: usize,
    most: usize,
    /// A job taken from `jobs` that didn't fit yet.
    waiting: Option<J>,
    /// The cost of each job out, in order.
    costs: VecDeque<usize>,
    /// What they add up to.
    held: usize,
    started: usize,
}

impl<'scope, J: Send, T: Send, I, C, W> Out<'scope, '_, J, T, I, C, W>
where
    I: Iterator<Item = J>,
    C: Fn(&J) -> usize,
    W: Fn(J) -> T + Sync,
{
    fn hand_out(&mut self) {
        while let Some(job) = self.waiting.take().or_else(|| self.jobs.next()) {
            let cost = (self.cost)(&job);
            let out = self.costs.len();
            // With nothing out, one goes, so a job dearer than the window still runs.
            if out > 0 && (out >= self.most || self.held.saturating_add(cost) > self.window) {
                self.waiting = Some(job);
                return;
            }
            if self.started < self.count.min(out + 1) {
                self.start_worker();
            }
            let mut state = self.shared.lock();
            let at = state.first + state.slots.len();
            state.queue.push_back((at, job));
            state.slots.push_back(None);
            drop(state);
            self.shared.job.notify_one();
            self.costs.push_back(cost);
            self.held = self.held.saturating_add(cost);
        }
    }

    fn start_worker(&mut self) {
        let (shared, work, hooks): (&'scope Shared<J, T>, &'scope W, _) =
            (self.shared, self.work, self.hooks.clone());
        let started = thread::Builder::new().spawn_scoped(self.scope, move || {
            hooks.set();
            let _lost = Lost(shared);
            while let Some((at, job)) = shared.next_job() {
                shared.put(at, work(job));
            }
        });
        match started {
            Ok(_) => self.started += 1,
            // The workers there are do the jobs, or this thread if there are none.
            Err(_) => self.count = self.started,
        }
    }

    /// The next result in the order, once it is in; `None` if a worker stopped part way.
    fn wait(&mut self) -> Option<T> {
        let mut state = self.shared.lock();
        loop {
            if state.slots.front().is_some_and(Option::is_some) {
                state.first += 1;
                return state.slots.pop_front().flatten();
            }
            if state.lost {
                return None;
            }
            if self.started == 0
                && let Some((at, job)) = state.queue.pop_front()
            {
                drop(state);
                let result = (self.work)(job);
                state = self.shared.lock();
                state.put(at, result);
                continue;
            }
            state = self
                .shared
                .result
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl<J: Send, T: Send, I, C, W> Iterator for Out<'_, '_, J, T, I, C, W>
where
    I: Iterator<Item = J>,
    C: Fn(&J) -> usize,
    W: Fn(J) -> T + Sync,
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.hand_out();
        self.costs.front()?;
        let result = self.wait()?;
        if let Some(cost) = self.costs.pop_front() {
            self.held = self.held.saturating_sub(cost);
        }
        Some(result)
    }
}

impl<J, T, I, C, W> Drop for Out<'_, '_, J, T, I, C, W>
where
    I: Iterator<Item = J>,
    C: Fn(&J) -> usize,
    W: Fn(J) -> T + Sync,
{
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.closed = true;
        let left = (
            std::mem::take(&mut state.queue),
            std::mem::take(&mut state.slots),
        );
        drop(state);
        self.shared.job.notify_all();
        drop(left);
    }
}

/// Wakes the thread taking results if its worker stops part way, which would otherwise wait for
/// a result that never comes. The scope then passes the worker's panic on.
struct Lost<'a, J, T>(&'a Shared<J, T>);

impl<J, T> Drop for Lost<'_, J, T> {
    fn drop(&mut self) {
        if thread::panicking() {
            self.0.lock().lost = true;
            self.0.result.notify_all();
        }
    }
}

/// What a test set on the thread that starts the workers, which each worker sets on itself.
#[derive(Clone)]
struct Hooks {
    #[cfg(feature = "fault-injection")]
    faults: crate::fsio::fault::Hooks,
    #[cfg(any(test, feature = "test-support"))]
    cut_sizes: Option<(u64, u64)>,
}

impl Hooks {
    fn here() -> Hooks {
        Hooks {
            #[cfg(feature = "fault-injection")]
            faults: crate::fsio::fault::hooks(),
            #[cfg(any(test, feature = "test-support"))]
            cut_sizes: crate::chunks::test_cut_sizes(),
        }
    }

    fn set(&self) {
        #[cfg(feature = "fault-injection")]
        crate::fsio::fault::set_hooks(&self.faults);
        #[cfg(any(test, feature = "test-support"))]
        crate::chunks::set_test_cut_sizes(self.cut_sizes);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    /// A pause that differs from job to job, so they finish out of order.
    fn pause(i: usize) {
        thread::sleep(Duration::from_micros((i * 7919 % 13) as u64 * 150));
    }

    #[test]
    fn results_come_back_in_the_jobs_order_whatever_the_timing() {
        for count in [1, 3, 8] {
            set_test_workers(Some(count));
            let mut got = Vec::new();
            let done = with_workers(|w| {
                let work = |i: usize| {
                    pause(i);
                    i * 2
                };
                w.in_order(
                    0..300,
                    WINDOW,
                    |_| 1,
                    work,
                    |r| {
                        got.push(r);
                        Ok::<_, ()>(())
                    },
                )
            });
            assert_eq!(done, Ok(()));
            assert_eq!(got, (0..300).map(|i| i * 2).collect::<Vec<_>>(), "{count}");
        }
        set_test_workers(None);
    }

    #[test]
    fn no_more_than_the_window_is_handed_out_however_slow_the_workers() {
        set_test_workers(Some(4));
        let costs: Vec<usize> = (0..200).map(|i| 1 + i * 37 % 10).collect();
        // A tight window, then one only the number of jobs a worker bounds.
        for window in [25, usize::MAX] {
            let taken = Cell::new(0);
            // A job is taken from the list only once every one before it is out: what is out
            // then, from the oldest not taken yet, must fit.
            let jobs = (0..costs.len()).inspect(|&i| {
                let out = &costs[taken.get()..i];
                assert!(out.iter().sum::<usize>() <= window, "{out:?} out");
                assert!(out.len() <= 4 * 4, "{} jobs out", out.len());
            });
            let slow = |i: usize| {
                thread::sleep(Duration::from_millis(1));
                i
            };
            with_workers(|w| {
                w.in_order(
                    jobs,
                    window,
                    |&i| costs[i],
                    slow,
                    |i| {
                        assert_eq!(i, taken.get());
                        taken.set(i + 1);
                        Ok::<_, ()>(())
                    },
                )
            })
            .unwrap();
            assert_eq!(taken.get(), costs.len());
        }
        set_test_workers(None);
    }

    #[test]
    fn a_slow_writer_never_blocks_a_worker() {
        set_test_workers(Some(2));
        let finished = AtomicUsize::new(0);
        let mut first = true;
        let work = |i: usize| {
            finished.fetch_add(1, Ordering::SeqCst);
            i
        };
        with_workers(|w| {
            w.in_order(
                0..8,
                WINDOW,
                |_| 1,
                work,
                |_| {
                    if first {
                        first = false;
                        // The writer holds on to the first result: the workers still finish every
                        // job out, with no one taking what they found.
                        let start = Instant::now();
                        while finished.load(Ordering::SeqCst) < 8 {
                            assert!(start.elapsed() < Duration::from_secs(20), "a worker waited");
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                    Ok::<_, ()>(())
                },
            )
        })
        .unwrap();
        set_test_workers(None);
    }

    #[test]
    fn a_job_dearer_than_the_window_still_goes_out_alone() {
        set_test_workers(Some(4));
        let mut got = Vec::new();
        with_workers(|w| {
            w.in_order(
                [1, 100, 1],
                10,
                |&c| c,
                |c| c,
                |c| {
                    got.push(c);
                    Ok::<_, ()>(())
                },
            )
        })
        .unwrap();
        assert_eq!(got, [1, 100, 1]);
        set_test_workers(None);
    }

    #[test]
    fn the_first_error_stops_the_jobs() {
        set_test_workers(Some(8));
        let mut taken = Vec::new();
        let stopped = with_workers(|w| {
            w.in_order(
                0..1000,
                WINDOW,
                |_| 1,
                |i| i,
                |i| {
                    taken.push(i);
                    if i == 10 { Err(i) } else { Ok(()) }
                },
            )
        });
        assert_eq!(stopped, Err(10));
        assert_eq!(taken, (0..=10).collect::<Vec<_>>());
        set_test_workers(None);
    }
}
