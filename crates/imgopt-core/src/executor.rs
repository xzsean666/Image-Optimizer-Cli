use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Instant;

use crate::error::ImgoptError;

/// Configuration for batch execution concurrency and error tolerance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchOptions {
    /// Maximum number of concurrent worker threads.
    pub concurrency: usize,
    /// Whether to abort remaining work immediately on first error. Default is false (isolate per-file errors).
    pub stop_on_error: bool,
}

impl Default for BatchOptions {
    fn default() -> Self {
        let default_concurrency = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            concurrency: default_concurrency.clamp(1, 16),
            stop_on_error: false,
        }
    }
}

/// The result of processing an individual item in a batch.
#[derive(Debug)]
pub struct BatchItemResult<T> {
    pub index: usize,
    pub source_path: PathBuf,
    pub output: Result<T, ImgoptError>,
}

/// Aggregated statistics of a batch execution run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchSummary {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub elapsed_millis: u64,
}

/// An input item submitted to the batch executor.
#[derive(Debug, Clone)]
pub struct BatchTask<I> {
    pub path: PathBuf,
    pub data: I,
}

impl<I> BatchTask<I> {
    pub fn new<P: AsRef<Path>>(path: P, data: I) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            data,
        }
    }
}

/// Execute a batch of tasks across a bounded scoped worker pool without requiring `'static`.
pub fn execute_batch_scoped<I, T, F>(
    tasks: Vec<BatchTask<I>>,
    options: &BatchOptions,
    worker: F,
) -> (Vec<BatchItemResult<T>>, BatchSummary)
where
    I: Send,
    T: Send,
    F: Fn(PathBuf, I) -> Result<T, ImgoptError> + Sync,
{
    let start_time = Instant::now();
    let total = tasks.len();

    if tasks.is_empty() {
        return (
            Vec::new(),
            BatchSummary {
                total: 0,
                succeeded: 0,
                failed: 0,
                elapsed_millis: 0,
            },
        );
    }

    let concurrency = options.concurrency.max(1).min(total);
    let indexed_tasks: VecDeque<(usize, BatchTask<I>)> = tasks.into_iter().enumerate().collect();
    let queue = Mutex::new(indexed_tasks);
    let has_error = AtomicBool::new(false);
    let stop_on_err = options.stop_on_error;
    let worker_ref = &worker;

    let mut results = Vec::with_capacity(total);
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..concurrency {
            let tx_clone = tx.clone();
            let queue_ref = &queue;
            let err_ref = &has_error;
            scope.spawn(move || loop {
                if stop_on_err && err_ref.load(Ordering::Acquire) {
                    break;
                }

                let next_item = {
                    let mut guard = match queue_ref.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.pop_front()
                };

                let Some((index, task)) = next_item else {
                    break;
                };

                let path = task.path.clone();
                let result = worker_ref(task.path, task.data);

                if result.is_err() && stop_on_err {
                    err_ref.store(true, Ordering::Release);
                }

                let _ = tx_clone.send(BatchItemResult {
                    index,
                    source_path: path,
                    output: result,
                });
            });
        }
        drop(tx);

        for item in rx {
            if item.output.is_ok() {
                succeeded += 1;
            } else {
                failed += 1;
            }
            results.push(item);
        }
    });

    results.sort_by_key(|r| r.index);

    let elapsed_millis = start_time.elapsed().as_millis() as u64;

    (
        results,
        BatchSummary {
            total,
            succeeded,
            failed,
            elapsed_millis,
        },
    )
}

/// Execute a batch of tasks across a bounded worker pool with per-item failure isolation.
pub fn execute_batch<I, T, F>(
    tasks: Vec<BatchTask<I>>,
    options: &BatchOptions,
    worker: F,
) -> (Vec<BatchItemResult<T>>, BatchSummary)
where
    I: Send + 'static,
    T: Send + 'static,
    F: Fn(PathBuf, I) -> Result<T, ImgoptError> + Sync + Send + 'static,
{
    execute_batch_scoped(tasks, options, worker)
}

/// Test probe helper to track peak concurrent active worker threads.
#[derive(Debug, Default)]
pub struct ConcurrencyProbe {
    current: AtomicUsize,
    peak: AtomicUsize,
}

impl ConcurrencyProbe {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_enter(&self) {
        let cur = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        let mut peak = self.peak.load(Ordering::SeqCst);
        while cur > peak {
            match self
                .peak
                .compare_exchange_weak(peak, cur, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(actual) => peak = actual,
            }
        }
    }

    pub fn on_exit(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorKind, PipelineStage};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn test_empty_batch() {
        let tasks: Vec<BatchTask<()>> = Vec::new();
        let (results, summary) =
            execute_batch(tasks, &BatchOptions::default(), |_path, _data| Ok(()));
        assert_eq!(results.len(), 0);
        assert_eq!(summary.total, 0);
    }

    #[test]
    fn test_batch_failure_isolation() {
        let tasks = vec![
            BatchTask::new("item1.jpg", 1),
            BatchTask::new("broken.jpg", 2),
            BatchTask::new("item3.jpg", 3),
        ];

        let options = BatchOptions {
            concurrency: 2,
            stop_on_error: false,
        };

        let (results, summary) = execute_batch(tasks, &options, |path, val| {
            if val == 2 {
                Err(ImgoptError::new(
                    PipelineStage::Decode,
                    ErrorKind::Decode("corrupted item".into()),
                )
                .with_path(path))
            } else {
                Ok(val * 10)
            }
        });

        assert_eq!(summary.total, 3);
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed, 1);

        assert_eq!(results[0].output.as_ref().unwrap(), &10);
        assert!(results[1].output.is_err());
        assert_eq!(results[2].output.as_ref().unwrap(), &30);
    }

    #[test]
    fn test_concurrency_bound_respected() {
        let probe = Arc::new(ConcurrencyProbe::new());
        let total_tasks = 8;
        let concurrency_limit = 3;

        let tasks: Vec<BatchTask<usize>> = (0..total_tasks)
            .map(|i| BatchTask::new(format!("task_{i}"), i))
            .collect();

        let options = BatchOptions {
            concurrency: concurrency_limit,
            stop_on_error: false,
        };

        let probe_clone = Arc::clone(&probe);
        let (_results, summary) = execute_batch(tasks, &options, move |_path, _i| {
            probe_clone.on_enter();
            thread::sleep(Duration::from_millis(20));
            probe_clone.on_exit();
            Ok(())
        });

        assert_eq!(summary.total, total_tasks);
        assert_eq!(summary.succeeded, total_tasks);
        assert!(
            probe.peak() <= concurrency_limit,
            "Peak concurrency {} must not exceed limit {}",
            probe.peak(),
            concurrency_limit
        );
    }
}
