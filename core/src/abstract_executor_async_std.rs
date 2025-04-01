
#[cfg(feature = "exec_async_std")]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, enabling seamless
    //! swapping of threading implementations (e.g., `nuclei` or `async-std`) in the future.
    //!
    //! It leverages the `async-std` crate, a lightweight async runtime, for asynchronous execution.
    //! The module offers utilities for initializing the executor, spawning detached tasks (both
    //! local and global), handling blocking operations, and synchronously blocking on futures.
    //! The design ensures flexibility and compatibility with `async-std`’s OS-backed async mechanisms.

    // ## Imports
    use std::future::Future; // Core trait for asynchronous operations.
    use std::io::Result; // Standard IO types for error handling.
    use std::thread; // For thread management in the driver loop.
    use std::time::Duration; // For timing operations in driver restart delay.
    use lazy_static::lazy_static; // For static initialization of `INIT`.
    use log::{error, trace, warn}; // Logging utilities for debugging and error reporting.
    use parking_lot::Once; // Synchronization primitive for one-time initialization.
    use crate::ProactorConfig; // Custom configuration enum, ignored in this impl.
    use std::panic::{catch_unwind, AssertUnwindSafe}; // Panic handling for driver robustness.
    use async_std::task; // Core async-std module for task spawning and execution.

    /// Spawns a future that can be sent across threads and detaches it for independent execution.
    ///
    /// This function uses `async_std::task::spawn` to schedule the future on the global multi-threaded
    /// executor. Ignoring the `Task` handle detaches it, allowing thread-safe tasks to run independently.
    pub fn spawn_detached<F: Future<Output=T> + Send + 'static, T: Send + 'static>(future: F) -> () {
        let _ = task::spawn(future);
    } // only 5x calls in metric_server and actor_builder

    /// Spawns a blocking task on a separate thread for CPU-bound or blocking operations.
    ///
    /// This async function uses `async_std::task::spawn_blocking` to run the closure on a thread pool,
    /// awaiting its result. Ideal for operations like file IO or heavy computation that shouldn’t block
    /// the async executor.
    pub async fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> T {
         task::spawn_blocking(f).await
    } //only once in metric_server

    /// Blocks the current thread until the provided future completes, returning its result.
    ///
    /// This function uses `async_std::task::block_on` to run the future to completion on the current
    /// thread, useful in synchronous contexts like `main` or tests where an async runtime isn’t running.
    pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
       // futures::executor::block_on(future)

        task::block_on(future)
    }  // 26 usages confirmed from sync code discovered everywhere

    /// Asynchronously spawns additional threads in the global executor.
    ///
    /// Since `async-std` does not support dynamically adding threads to its executor, this function
    /// returns `Ok(0)` as a no-op, maintaining interface compatibility with the `nuclei` version.
    pub async fn spawn_more_threads(count: usize) -> Result<usize> {
        Ok(0) // async-std doesn’t allow dynamic thread addition
    }

    lazy_static! {
        /// Ensures initialization runs only once across all threads using a thread-safe static.
        static ref INIT: Once = Once::new();
    }

    /// Initializes the `async-std` executor with the specified configuration.
    ///
    /// This function optionally starts a driver thread that runs the `async-std` executor indefinitely
    /// if `enable_driver` is true. The `proactor_config` and `queue_length` parameters are ignored since
    /// `async-std` doesn’t use `nuclei`’s io_uring-based proactor model. Includes panic handling for robustness.
    pub(crate) fn init(enable_driver: bool, _proactor_config: ProactorConfig, _queue_length: u32) {
        INIT.call_once(|| {
            //if enable_driver {
                trace!("Starting async-std driver");
                thread::spawn(|| {
                    loop {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            // Run the async-std executor indefinitely with a pending future
                            task::block_on(std::future::pending::<()>());
                        }));
                        if let Err(e) = result {
                            error!("async-std driver panicked: {:?}", e);
                            thread::sleep(Duration::from_secs(1));
                            warn!("Restarting async-std driver");
                        }
                    }
                });
           // }
        });
    }
}