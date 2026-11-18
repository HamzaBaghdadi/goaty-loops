//! A small worker pool for the audio thread.
//!
//! rayon's workers spin between jobs; fed every few milliseconds by the audio
//! callback they never go to sleep, so a project with a handful of effect
//! tracks kept 5-6 cores busy doing nothing. Here workers spin only briefly
//! (enough to catch the next routing level of the same block), then park until
//! the next block. Dispatching does not allocate.

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{JoinHandle, Thread};
use std::time::{Duration, Instant};

/// How long an idle worker keeps polling before it parks.
const SPIN: Duration = Duration::from_micros(50);

type Task<'a> = &'a (dyn Fn(usize) + Sync);

/// `state` packs the generation (high 16 bits), the task count (next 16) and
/// the next unclaimed index (low 32), so a worker always claims an index of a
/// generation whose count it read in the same atomic operation.
fn pack(generation: u64, total: u64, next: u64) -> u64 {
    (generation & 0xffff) << 48 | (total & 0xffff) << 32 | next
}
fn unpack(v: u64) -> (u64, u64, u64) {
    (v >> 48, (v >> 32) & 0xffff, v & 0xffff_ffff)
}

struct Inner {
    state: AtomicU64,
    /// Points at the caller's `Task` (a fat reference on its stack). Only read
    /// after successfully claiming an index, while the caller still waits.
    task: AtomicPtr<()>,
    done: AtomicUsize,
    shutdown: AtomicBool,
}

pub struct WorkerPool {
    inner: Arc<Inner>,
    threads: Vec<Thread>,
    handles: Vec<JoinHandle<()>>,
    generation: u64,
}

impl WorkerPool {
    /// A pool with `workers` helper threads (the caller also works).
    pub fn new(workers: usize) -> Self {
        let inner = Arc::new(Inner {
            state: AtomicU64::new(0),
            task: AtomicPtr::new(std::ptr::null_mut()),
            done: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
        });
        let mut handles = Vec::with_capacity(workers);
        for i in 0..workers {
            let inner = inner.clone();
            let spawned = std::thread::Builder::new().name(format!("mixer worker {i}")).spawn(move || worker(&inner));
            match spawned {
                Ok(h) => handles.push(h),
                Err(_) => break,
            }
        }
        let threads = handles.iter().map(|h| h.thread().clone()).collect();
        Self { inner, threads, handles, generation: 0 }
    }

    /// Helper threads to use on this machine: one fewer than the cores, at most 7.
    pub fn default_workers() -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get()).saturating_sub(1).clamp(1, 7)
    }

    /// Runs `task(0..count)`, spread over the caller and the workers, and
    /// returns once every call has finished.
    pub fn run(&mut self, count: usize, task: Task<'_>) {
        if count <= 1 || self.threads.is_empty() {
            (0..count).for_each(task);
            return;
        }
        let count = count.min(0xffff);
        let inner = &*self.inner;
        self.generation = (self.generation + 1) & 0xffff;
        let task_ref: Task<'_> = task;
        inner.task.store(&task_ref as *const Task<'_> as *mut (), Ordering::Relaxed);
        inner.done.store(0, Ordering::Relaxed);
        inner.state.store(pack(self.generation, count as u64, 0), Ordering::Release);
        for t in self.threads.iter().take(count - 1) {
            t.unpark();
        }
        while let Some(i) = claim(inner, self.generation) {
            task(i);
            inner.done.fetch_add(1, Ordering::Release);
        }
        // The remaining tasks are running on workers; they finish soon.
        let mut spins = 0u32;
        while inner.done.load(Ordering::Acquire) < count {
            spins += 1;
            if spins < 4096 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::Release);
        for t in &self.threads {
            t.unpark();
        }
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

/// Claims the next index of generation `generation`, if any is left.
fn claim(inner: &Inner, generation: u64) -> Option<usize> {
    let mut v = inner.state.load(Ordering::Acquire);
    loop {
        let (g, total, next) = unpack(v);
        if g != generation || next >= total {
            return None;
        }
        match inner.state.compare_exchange_weak(v, v + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(next as usize),
            Err(now) => v = now,
        }
    }
}

fn worker(inner: &Inner) {
    boost_priority();
    let mut seen = 0u64;
    loop {
        // Wait for a new generation: spin briefly, then park.
        let started = Instant::now();
        let generation = loop {
            if inner.shutdown.load(Ordering::Acquire) {
                return;
            }
            let (g, _, _) = unpack(inner.state.load(Ordering::Acquire));
            if g != seen {
                break g;
            }
            if started.elapsed() < SPIN {
                std::hint::spin_loop();
            } else {
                std::thread::park();
            }
        };
        seen = generation;
        while let Some(i) = claim(inner, generation) {
            // SAFETY: an index was claimed for this generation, so its caller is
            // still inside `run` (it waits for `done`), keeping the task alive.
            let task = unsafe { *(inner.task.load(Ordering::Acquire) as *const Task<'_>) };
            task(i);
            inner.done.fetch_add(1, Ordering::Release);
        }
    }
}

/// Mixer workers render audio, so they get the same urgency as the audio thread.
#[cfg(windows)]
fn boost_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }
}

#[cfg(not(windows))]
fn boost_priority() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_every_index_once() {
        let mut pool = WorkerPool::new(3);
        for round in 0..2000 {
            let hits: Vec<AtomicUsize> = (0..17).map(|_| AtomicUsize::new(0)).collect();
            let n = 1 + round % 17;
            pool.run(n, &|i| {
                hits[i].fetch_add(1, Ordering::Relaxed);
            });
            for (i, h) in hits.iter().enumerate() {
                assert_eq!(h.load(Ordering::Relaxed), usize::from(i < n), "round {round} index {i}");
            }
        }
    }

    #[test]
    fn shuts_down_cleanly() {
        let pool = WorkerPool::new(2);
        std::thread::sleep(Duration::from_millis(20));
        drop(pool); // joins: proves workers wake on shutdown
    }
}
