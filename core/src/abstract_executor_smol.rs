#[cfg(feature = "exec_smol")]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, allowing for easier
    //! swapping of threading implementations in the future.
    //!
    //! The module leverages the `smol` library for asynchronous execution and provides
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
    use smol::{Task, unblock};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Spawns a local task intended for lightweight operations.
    ///
    /// Note: In `smol`, tasks are scheduled on the global executor, which may not guarantee
    /// execution on the current thread, unlike `nuclei::spawn_local`.
    pub fn spawn_local<F: Future<Output = T> + 'static, T: 'static>(f: F) -> Task<T> {
        smol::spawn(f)
    }

    /// Spawns a blocking task on a separate thread for CPU-bound or blocking operations.
    pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> Task<T> {
        smol::spawn(unblock(f))
    }

    /// Spawns a task that can run on any thread in the pool for parallel execution.
    pub fn spawn<F: Future<Output = T> + Send + 'static, T: Send + 'static>(future: F) -> Task<T> {
        smol::spawn(future)
    }

    /// Asynchronously spawns additional threads in the executor.
    ///
    /// Note: `smol` does not support dynamically adding threads at runtime, so this returns an error.
    pub async fn spawn_more_threads(_count: usize) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Dynamic thread spawning not supported in smol",
        ))
    }

    lazy_static! {
        /// Ensures initialization runs only once across all threads.
        static ref INIT: Once = Once::new();
    }

    /// Initializes the `smol` executor with the specified configuration.
    ///
    /// If `enable_driver` is true, spawns a dedicated thread to run the executor indefinitely.
    /// The `proactor_config` and `queue_length` parameters are ignored as `smol` does not use
    /// `io_uring` or similar configurable proactors.
    pub(crate) fn init(enable_driver: bool, _proactor_config: ProactorConfig, _queue_length: u32) {
        INIT.call_once(|| {
            if enable_driver {
                trace!("Starting smol driver");
                std::thread::spawn(|| {
                    loop {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            block_on(std::future::pending::<()>());
                        }));
                        if let Err(e) = result {
                            error!("Smol driver panicked: {:?}", e);
                            thread::sleep(Duration::from_secs(1));
                            warn!("Restarting smol driver");
                        }
                    }
                });
            }
        });
    }

    /// Blocks the current thread until the given future completes.
    pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {

        smol::block_on(future)
    }
}

