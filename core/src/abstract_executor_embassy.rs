#[cfg(any(feature = "exec_embassy"))]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, allowing for easier
    //! swapping of threading implementations in the future.
    //!
    //! The module leverages the `embassy_executor` library for asynchronous execution and provides
    //! utilities for initialization, spawning detached tasks, and blocking on futures.

    use std::future::Future;
    use std::io::{self, Result};
    use lazy_static::lazy_static;
    use parking_lot::Once;
    use crate::ProactorConfig;
    use embassy_executor::{Spawner};

    /// Spawns a local task intended for lightweight operations.
    ///
    /// In Embassy, tasks are scheduled on the executor without distinguishing between
    /// local and pooled tasks. The task must be `'static` and produce a `'static` output.
    pub fn spawn_local<F, T>(f: F) -> embassy_executor::Task<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        let spawner = embassy_executor::Spawner::for_current_executor();
        spawner.spawn(f)
    }

    /// Spawns a blocking task on a separate thread for CPU-bound or blocking operations.
    ///
    /// Embassy does not natively support blocking operations. This implementation simulates
    /// blocking by running the closure in an async task, which may not be ideal for CPU-bound work.
    pub fn spawn_blocking<F, T>(f: F) -> embassy_executor::Task<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let spawner = embassy_executor::Spawner::for_current_executor();
        spawner.spawn(async move { f() })
    }

    /// Spawns a task that can run on any thread in the pool for parallel execution.
    ///
    /// In Embassy, this is identical to `spawn_local` as there’s no thread pool distinction.
    pub fn spawn<F, T>(future: F) -> embassy_executor::Task<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let spawner = embassy_executor::Spawner::for_current_executor();
        spawner.spawn(future)
    }

    /// Asynchronously spawns additional threads in the executor.
    ///
    /// Embassy does not support dynamic thread spawning, so this function returns an error.
    pub async fn spawn_more_threads(_count: usize) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Dynamic thread spawning not supported in Embassy",
        ))
    }

    lazy_static! {
        /// Ensures initialization runs only once across all threads.
        static ref INIT: Once = Once::new();
    }

    /// Initializes the Embassy executor with the specified configuration.
    ///
    /// If `enable_driver` is true, starts the executor in a dedicated thread.
    /// The `proactor_config` and `queue_length` parameters are ignored as Embassy does not use
    /// `io_uring` or similar configurable proactors.
    pub(crate) fn init(enable_driver: bool, _proactor_config: ProactorConfig, _queue_length: u32) {
        INIT.call_once(|| {
            if enable_driver {
                trace!("Starting Embassy driver");
                std::thread::spawn(|| {
                    let executor = Executor::new();
                    executor.run(|_spawner| {
                        // Embassy's executor runs indefinitely; no tasks are spawned here
                        // as the spawner is typically used elsewhere in the application.
                    });
                });
            }
        });
    }

    /// Blocks the current thread until the given future completes.
    ///
    /// Embassy is not designed for blocking on a single future. This implementation runs
    /// a new executor instance until the future completes, which may be inefficient.
    pub fn block_on<F, T>(future: F) -> T
    where
        F: Future<Output = T>,
        T: 'static,
    {
        let mut result = None;
        let executor = Executor::new();
        executor.run(|spawner| {
            spawner.spawn(async {
                result = Some(future.await);
            })
        });
        result.expect("Future did not complete")
    }
}