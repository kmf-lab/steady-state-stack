
#[cfg(feature = "exec_async_std")]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, enabling seamless
    //! swapping of threading implementations (e.g., `nuclei` or `async-std`) in the future.
    //!
    //! It leverages the `async-std` crate, a lightweight async runtime, for asynchronous execution.
    //! The module offers utilities for initializing the executor, spawning detached tasks (both
    //! local and global), handling blocking operations, and synchronously blocking on futures.
    //! The design ensures flexibility and compatibility with `async-std`’s OS-backed async mechanisms.

    use std::error::Error;
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
    use futures::channel::oneshot;

    /// Spawns a future that can be sent across threads and detaches it for independent execution.
    ///
    /// This function uses `async_std::task::spawn` to schedule the future on the global multi-threaded
    /// executor. Ignoring the `Task` handle detaches it, allowing thread-safe tasks to run independently.
    pub fn spawn_detached<F: Future<Output=T> + Send + 'static, T: Send + 'static>(future: F) {
        let _ = task::spawn(future);
    } // only 5x calls in metric_server and actor_builder



    // Get the current core (platform-specific)
    #[cfg(all(unix, feature = "libc"))]
    fn get_current_core() -> Option<usize> {
        let cpu = unsafe { libc::sched_getcpu() };
        if cpu >= 0 {
            Some(cpu as usize)
        } else {
            None
        }
    }

    #[cfg(all(windows, feature = "winapi"))]
    fn get_current_core() -> Option<usize> {
        let cpu = unsafe { winapi::um::processthreadsapi::GetCurrentProcessorNumber() };
        if cpu != 0xFFFFFFFF {
            Some(cpu as usize)
        } else {
            None
        }
    }

    #[cfg(not(any(all(unix, feature = "libc"), all(windows, feature = "winapi"))))]
    fn get_current_core() -> Option<usize> {
        None
    }

    // Set thread affinity (platform-specific)
    #[cfg(all(unix, feature = "libc"))]
    fn set_thread_affinity(core: usize) -> Result<(), dyn Box<Error>> {
        use libc::{cpu_set_t, pthread_setaffinity_np, pthread_self};
        let mut cpu_set: cpu_set_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::CPU_SET(core, &mut cpu_set);
            let res = pthread_setaffinity_np(pthread_self(), std::mem::size_of::<cpu_set_t>(), &cpu_set);
            if res == 0 {
                Ok(())
            } else {
                Err(())
            }
        }
    }

    #[cfg(all(windows, feature = "winapi"))]
    fn set_thread_affinity(core: usize) -> std::result::Result<(), Box<dyn Error>> {
        use winapi::um::processthreadsapi::GetCurrentThread;
        use winapi::shared::basetsd::DWORD_PTR;

        let mask = 1u64 << core;
        let res = unsafe { winapi::um::winbase::SetThreadAffinityMask(GetCurrentThread(), mask as DWORD_PTR) };
        if res != 0 {
            Ok(())
        } else {
            Err("unable to set affinity on windows due to mask failure".into())
        }
    }

    #[cfg(not(any(all(unix, feature = "libc"), all(windows, feature = "winapi"))))]
    fn set_thread_affinity(_core: usize) -> std::result::Result<(), Box<dyn Error>> {
        Ok(())
    }

    /// Spawns a blocking task on a new thread with optional core affinity.
    ///
    /// This function runs the closure `f` on a separate thread. If the platform-specific feature
    /// (`libc` on Unix, `winapi` on Windows) is enabled, it sets the thread's core affinity to
    /// match the calling thread's core. It returns a future that resolves to the result of `f`.
    ///
    /// # Arguments
    /// * `f` - A closure that performs a blocking operation and returns a value of type `T`.
    ///
    /// # Returns
    /// A future that can be awaited to obtain the result of `f`.
    pub fn spawn_blocking<F, T>(f: F) -> impl Future<Output = T> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let current_core = get_current_core();
        let (sender, receiver) = oneshot::channel();

        thread::spawn(move || {
            if let Some(core) = current_core {
                if let Err(e) = set_thread_affinity(core) {
                    warn!("Affinity for blocking tasks was enabled but unable to set due to '{:?}', will run blocking task on another core.",e)
                }
            }
            if let Err(_e) = sender.send(f()) {
                //may happen as expected in some shutdown cases
                warn!("blocking job finished but the receiver is no longer attached");
            }
        });

        async move {
            receiver.await.expect("Sender dropped")
        }
    }

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
    pub async fn spawn_more_threads(_count: usize) -> Result<usize> {
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
            if enable_driver {
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
            }
        });
    }
}

// Additional tests for async-std executor abstractions
#[cfg(test)]
mod async_std_exec_tests {
    use std::{thread, time::Duration};
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    use crate::abstract_executor_async_std::core_exec::*;

    #[test]
    fn test_spawn_detached_async_std() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();
        spawn_detached(async move { flag_clone.store(true, Ordering::SeqCst); });
        thread::sleep(Duration::from_millis(50));
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_spawn_blocking_async_std() {
        let result = block_on(async { spawn_blocking(|| 9).await });
        assert_eq!(result, 9);
    }

    #[test]
    fn test_spawn_more_threads_async_std() {
        let result = block_on(async { spawn_more_threads(3).await.expect("internal error") });
        assert_eq!(result, 0);
    }
}