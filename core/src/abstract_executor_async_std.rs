
#[cfg(any(feature = "exec_async_std", feature = "exec_async_std"))]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, allowing for easier
    //! swapping of threading implementations in the future.
    //!
    //! The module leverages the `async_std` library for asynchronous execution and provides
    //! utilities for initialization, spawning detached tasks, and blocking on futures.

    // ## Imports
    use std::future::Future;
    use std::io::{self, Result};
    use std::thread;
    use std::time::Duration;
    use lazy_static::lazy_static;
    use log::{error, trace, warn};
    use parking_lot::Once;
    use crate::ProactorConfig;
    use async_std::task::{self, JoinHandle};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Spawns a local task intended for lightweight operations.
    ///
    /// Note: `async_std` uses a thread pool, so this spawns on the global executor rather than
    /// strictly on the current thread.
    pub fn spawn_local<F: Future<Output = T> + 'static, T: 'static>(f: F) -> JoinHandle<T> {
        task::spawn(f)
    }

    /// Spawns a blocking task on a separate thread for CPU-bound or blocking operations.
    pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> JoinHandle<T> {
        task::spawn_blocking(f)
    }

    /// Spawns a task that can run on any thread in the pool for parallel execution.
    pub fn spawn<F: Future<Output = T> + Send + 'static, T: Send + 'static>(future: F) -> JoinHandle<T> {
        task::spawn(future)
    }

    /// Asynchronously spawns additional threads in the executor.
    ///
    /// Note: `async_std` does not support dynamically adding threads at runtime, so this returns an error.
    pub async fn spawn_more_threads(_count: usize) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "async_std does not support dynamic thread spawning",
        ))
    }

    lazy_static! {
        /// Ensures initialization runs only once across all threads.
        static ref INIT: Once = Once::new();
    }

    /// Initializes the `async_std` executor with the specified configuration.
    ///
    /// If `enable_driver` is true, spawns a blocking task to keep the executor running indefinitely.
    /// The `proactor_config` and `queue_length` parameters are ignored as `async_std` does not use
    /// `io_uring` or similar configurable proactors.
    pub(crate) fn init(enable_driver: bool, _proactor_config: ProactorConfig, _queue_length: u32) {
        INIT.call_once(|| {
            if enable_driver {
                trace!("Starting async_std driver");
                task::spawn_blocking(|| {
                    loop {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            task::block_on(std::future::pending::<()>());
                        }));
                        if let Err(e) = result {
                            error!("async_std driver panicked: {:?}", e);
                            thread::sleep(Duration::from_secs(1));
                            warn!("Restarting async_std driver");
                        }
                    }
                }).detach();
            }
            // No specific executor configuration needed for async_std
        });
    }

    /// Blocks the current thread until the given future completes.
    pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
        task::block_on(future)
    }
}