//! A small worker pool for slow loads (decoding, PDF rendering). Jobs run
//! newest first, so whatever just scrolled into view loads before what
//! scrolled past; finished results wake the UI thread.

use std::collections::VecDeque;
use std::hash::Hash;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use ui::Waker;

type Job<T> = Box<dyn FnOnce() -> T + Send>;

struct Queue<K, T> {
    jobs: VecDeque<(K, Job<T>)>,
    closed: bool,
}

struct Shared<K, T> {
    queue: Mutex<Queue<K, T>>,
    ready: Condvar,
}

pub struct Loader<K, T> {
    shared: Arc<Shared<K, T>>,
    results: Receiver<(K, T)>,
    tx: Sender<(K, T)>,
    /// Run jobs on the calling thread (offscreen snapshots).
    inline: bool,
}

impl<K: Clone + Eq + Hash + Send + 'static, T: Send + 'static> Loader<K, T> {
    pub fn new(threads: usize, waker: Waker) -> Self {
        let shared = Arc::new(Shared { queue: Mutex::new(Queue { jobs: VecDeque::new(), closed: false }), ready: Condvar::new() });
        let (tx, results) = channel();
        for i in 0..threads.max(1) {
            let shared = shared.clone();
            let tx: Sender<(K, T)> = tx.clone();
            let waker = waker.clone();
            std::thread::Builder::new()
                .name(format!("loader-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let mut q = shared.queue.lock().unwrap();
                            loop {
                                if q.closed {
                                    return;
                                }
                                if let Some(job) = q.jobs.pop_back() {
                                    break job;
                                }
                                q = shared.ready.wait(q).unwrap();
                            }
                        };
                        let (key, f) = job;
                        if tx.send((key, f())).is_err() {
                            return;
                        }
                        waker.wake();
                    }
                })
                .expect("spawn loader thread");
        }
        Self { shared, results, tx, inline: false }
    }

    /// Runs every job right away on the caller's thread instead.
    pub fn set_inline(&mut self, inline: bool) {
        self.inline = inline;
    }

    pub fn request(&self, key: K, job: impl FnOnce() -> T + Send + 'static) {
        if self.inline {
            let _ = self.tx.send((key, job()));
            return;
        }
        let mut q = self.shared.queue.lock().unwrap();
        q.jobs.push_back((key, Box::new(job)));
        drop(q);
        self.shared.ready.notify_one();
    }

    /// Drops queued jobs whose key fails `keep`, returning their keys.
    pub fn retain_pending(&self, keep: impl Fn(&K) -> bool) -> Vec<K> {
        let mut q = self.shared.queue.lock().unwrap();
        let mut dropped = Vec::new();
        q.jobs.retain(|(k, _)| {
            let ok = keep(k);
            if !ok {
                dropped.push(k.clone());
            }
            ok
        });
        dropped
    }

    /// Finished jobs since the last call.
    pub fn take_results(&self) -> Vec<(K, T)> {
        self.results.try_iter().collect()
    }
}

impl<K, T> Drop for Loader<K, T> {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap().closed = true;
        self.shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_cancels() {
        let mut l: Loader<u32, u32> = Loader::new(1, Waker::default());
        l.set_inline(true);
        l.request(2, || 4);
        assert_eq!(l.take_results(), vec![(2, 4)]);
        l.set_inline(false);
        let start = std::time::Instant::now();
        // Block the single worker so later jobs stay queued.
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let g = gate.clone();
        l.request(0, move || {
            drop(g.lock());
            0
        });
        while l.shared.queue.lock().unwrap().jobs.len() > 0 && start.elapsed().as_secs() < 5 {
            std::thread::yield_now();
        }
        l.request(1, || 1);
        l.request(3, || 3);
        assert_eq!(l.retain_pending(|k| *k != 1), vec![1]);
        drop(held);
        let mut got = Vec::new();
        while got.len() < 2 && start.elapsed().as_secs() < 5 {
            got.extend(l.take_results());
        }
        got.sort();
        assert_eq!(got, vec![(0, 0), (3, 3)]);
    }
}
