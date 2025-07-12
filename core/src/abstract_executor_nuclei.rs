use std::error::Error;
use futures::channel::oneshot;
use futures::FutureExt;

#[cfg(any(feature = "proactor_nuclei", feature = "proactor_tokio"))]
pub(crate) mod core_exec {
    //! This module provides an abstraction layer for threading solutions, enabling seamless
    //! swapping of threading implementations (e.g., `nuclei` or `tokio`) in the future.
    //!
    //! It leverages the `nuclei` crate, a runtime-agnostic proactive IO system, for asynchronous
    //! execution. The module offers utilities for initializing the executor, spawning detached
    //! tasks (both local and global), handling blocking operations, and synchronously blocking
    //! on futures. The design ensures flexibility and performance, particularly with `nuclei`'s
    //! io_uring backend on Linux.

    // ## Imports
    use std::net::{SocketAddr, TcpListener}; // For network-related operations, unused here but likely for future expansion.
    use std::future::Future; // Core trait for asynchronous operations.
    use std::io::{self, Result}; // Standard IO types for error handling.
    use std::pin::Pin; // For pinning futures in memory, required by the `Future` trait.
    use std::sync::Arc; // Atomic reference counting for thread-safe sharing, unused here but potentially for future use.
    use std::task::{Context, Poll}; // Core components of Rust's async runtime for polling futures.
    use std::thread; // For thread management, used in `InfiniteSleep` and driver loop.
    use std::time::Duration; // For timing operations, used in driver restart delay.
    use bytes::BytesMut; // Efficient byte buffer, unused here but likely for IO operations elsewhere.
    use lazy_static::lazy_static; // For static initialization of `INIT`, ensuring thread-safe setup.
    use log::{error, trace, warn}; // Logging utilities for debugging and error reporting.
    use nuclei::config::{IoUringConfiguration, NucleiConfig}; // `nuclei`-specific configuration for io_uring.
    use parking_lot::Once; // High-performance synchronization primitive for one-time initialization.
    use crate::ProactorConfig; // Custom configuration enum for selecting proactor modes.
    use futures::{AsyncRead, AsyncWrite}; // Traits for asynchronous IO, unused here but likely for compatibility.
    use futures_util::AsyncWriteExt; // Extensions for `AsyncWrite`, unused but potentially for future IO tasks.
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use futures_util::future::FusedFuture;
    use futures::FutureExt;
    // Panic handling for robustness in the driver loop.

    /// Spawns a future that can be sent across threads and detaches it for independent execution.
    ///
    /// Unlike `spawn_local_and_detach`, this function requires `Send` bounds, allowing the future
    /// and its output to be safely transferred between threads. It uses `nuclei::spawn` to schedule
    /// the task on the global multi-threaded executor, with `detach` ensuring it runs without
    /// returning a handle. Useful for tasks requiring thread mobility.
    pub fn spawn_detached<F: Future<Output=T> + Send + 'static, T: Send + 'static>(future: F) -> () {
        nuclei::spawn(future).detach();
    }


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
    fn set_thread_affinity(core: usize) -> std::io::Result<(), dyn Box<Error>> {
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
    fn set_thread_affinity(_core: usize) -> std::result::Result<(), Box<dyn std::error::Error>> {
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
    pub fn spawn_blocking<F, T>(f: F) -> Pin<Box<dyn futures::future::FusedFuture<Output = T> + Send>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let current_core = get_current_core();
        let (sender, receiver) = crate::oneshot::channel();

        thread::spawn(move || {
            if let Some(core) = current_core {
                if let Err(e) = set_thread_affinity(core) {
                    warn!("Affinity for blocking tasks was enabled but unable to set due to '{:?}', will run blocking task on another core.",e)
                }
            }
            if let Err(e) = sender.send(f()) {
                //may happen as expected in some shutdown cases
                warn!("blocking job finished but the receiver is no longer attached");
            }
        });

        Box::pin(async move {
            receiver.await.expect("Sender dropped")
        }.fuse()) as Pin<Box<dyn FusedFuture<Output = T> + Send>>
    }


    /// Blocks the current thread until the provided future completes, returning its result.
    ///
    /// This synchronous function runs the `nuclei` global and local executors on the current thread,
    /// driving the future to completion. It’s useful in contexts like `main` or tests where async
    /// runtime isn’t otherwise available. The lack of `Send` bounds allows flexibility for non-thread-safe
    /// futures, but it blocks the calling thread entirely.
    pub fn block_on<F: Future<Output = T>, T>(future: F) -> T {
        nuclei::block_on(future)
    }

    /// Asynchronously spawns additional threads in the global executor to handle increased load.
    ///
    /// Without this, task spawning might be throttled under heavy load. This function calls
    /// `nuclei::spawn_more_threads`, which increases the global executor’s thread count up to a
    /// configured maximum, returning the number of threads spawned or an error. For some executors,
    /// this might be a no-op, but with `nuclei`, it enhances scalability for IO-bound workloads.
    pub async fn spawn_more_threads(count: usize) -> Result<usize> {
        nuclei::spawn_more_threads(count).await
    }

    lazy_static! {
        /// Ensures initialization runs only once across all threads using a thread-safe static.
        ///
        /// The `Once` from `parking_lot` guarantees that the `init` function’s setup logic executes
        /// exactly once, even in a multi-threaded environment, preventing redundant or conflicting
        /// initialization of the `nuclei` executor.
        static ref INIT: Once = Once::new();
    }

    /// Initializes the `nuclei` executor with the specified configuration.
    ///
    /// This function sets up the `nuclei` proactor with a chosen `ProactorConfig` mode and queue length,
    /// optionally starting a driver thread. It uses `INIT.call_once` to ensure thread-safe, one-time
    /// execution. The setup is `nuclei`-specific, leveraging io_uring configurations for efficient IO
    /// handling on Linux.
    pub(crate) fn init(enable_driver: bool, proactor_config: ProactorConfig, queue_length: u32) {
        INIT.call_once(|| {
            // THIS ENTIRE BLOCK IS ONLY FOR nuclei. {}
            // FOR OTHER executors (e.g., smol), this may be empty or contain executor-specific init.

            /// A future that parks the thread indefinitely to keep the executor running without CPU usage.
            ///
            /// `InfiniteSleep` is used in the driver loop to maintain a thread that processes IO events
            /// via `nuclei::drive`. It wakes itself via the waker and parks the thread, ensuring the
            /// executor remains active without busy-waiting, a clever optimization for resource efficiency.
            struct InfiniteSleep;

            impl Future for InfiniteSleep {
                type Output = (); // No output, as it never completes.

                /// Polls the future, waking itself and parking the thread.
                ///
                /// This implementation ensures the thread remains alive, ready to process tasks, by
                /// waking the waker (to reschedule itself) and then parking (yielding control). It
                /// always returns `Poll::Pending`, never completing, which keeps the driver loop running.
                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                    cx.waker().wake_by_ref(); // Reschedule this future.
                    trace!("InfiniteSleep started"); // Log for debugging.
                    thread::park(); // Yield control, waiting for an unpark signal.
                    Poll::Pending // Never resolve, keeping the thread alive.
                }
            }

            /// Configures the `nuclei` proactor based on the provided `ProactorConfig`.
            ///
            /// Maps `ProactorConfig` variants to `IoUringConfiguration` methods, setting up io_uring
            /// with the specified queue length. Each mode tunes io_uring behavior (e.g., interrupt-driven
            /// vs. polling) for different performance characteristics.
            let nuclei_config = match proactor_config {
                ProactorConfig::InterruptDriven => IoUringConfiguration::interrupt_driven(queue_length),
                // Default io_uring mode: completions trigger interrupts.
                ProactorConfig::KernelPollDriven => IoUringConfiguration::kernel_poll_only(queue_length),
                // Likely uses IORING_SETUP_SQPOLL for kernel polling of submissions.
                ProactorConfig::LowLatencyDriven => IoUringConfiguration::low_latency_driven(queue_length),
                // Optimizes for low latency, possibly with IORING_SETUP_IOPOLL.
                ProactorConfig::IoPoll => IoUringConfiguration::io_poll(queue_length),
                // Polling-based IO, reducing interrupt overhead.
            };

            /// Initializes the `nuclei` proactor with the configured io_uring settings.
            ///
            /// The result is ignored (`let _ = ...`) as the proactor is a global singleton in `nuclei`,
            /// and this call establishes it for subsequent task execution.
            let _ = nuclei::Proactor::with_config(NucleiConfig { iouring: nuclei_config });

            /// Optionally starts a driver thread if `enable_driver` is true.
            ///
            /// This block spawns a blocking task that runs an infinite loop, driving the `InfiniteSleep`
            /// future to process IO events. It includes panic handling for robustness, restarting the
            /// driver after a delay if it crashes.
            if enable_driver {
                trace!("Starting IOUring driver"); // Log driver startup.
                nuclei::spawn_blocking(|| {
                    loop {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            // Drive the executor with `InfiniteSleep`, processing IO events.
                            nuclei::drive(InfiniteSleep);
                        }));
                        if let Err(e) = result {
                            error!("IOUring Driver panicked: {:?}", e); // Log panic details.
                            thread::sleep(Duration::from_secs(1)); // Delay before restart.
                            warn!("Restarting IOUring driver"); // Warn about restart.
                        }
                    }
                }).detach(); // Detach the task to run independently.
            }
            // END of nuclei-specific setup.
        });
    }
}